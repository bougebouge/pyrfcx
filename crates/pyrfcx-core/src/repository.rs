// Offline metadata operations: save/load repository files, create
// function containers from cached metadata for offline round-trip testing.

use std::path::Path;

use indexmap::IndexMap;

use crate::chunked::{self, ChunkedOutput};
use crate::connection::{CompatOptions, FuncHandle};
use crate::error::RfcError;
use crate::ffi;
use crate::marshal;
use crate::uc;
use crate::value::RfcValue;

// ── CFile: RAII wrapper for *mut FILE ─────────────────────────────────
//
// Uses std::fs::File for path handling and error reporting, then
// converts to FILE* via platform CRT for the SDK's RfcSaveRepository /
// RfcLoadRepository which require C stdio streams.

struct CFile(*mut ffi::FILE);

impl CFile {
    fn create(path: &Path) -> Result<Self, RfcError> {
        let file = std::fs::File::create(path).map_err(|e| {
            RfcError::InvalidInput(format!("cannot create {}: {e}", path.display()))
        })?;
        let ptr = file_to_cfile(file, c"wb")?;
        Ok(CFile(ptr))
    }

    fn open(path: &Path) -> Result<Self, RfcError> {
        let file = std::fs::File::open(path)
            .map_err(|e| RfcError::InvalidInput(format!("cannot open {}: {e}", path.display())))?;
        let ptr = file_to_cfile(file, c"rb")?;
        Ok(CFile(ptr))
    }

    fn as_ptr(&self) -> *mut ffi::FILE {
        self.0
    }
}

impl Drop for CFile {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // Safety: self.0 is a valid FILE* obtained from fdopen/_fdopen.
            // fclose closes both the stream and the underlying fd/handle.
            unsafe {
                fclose(self.0);
            }
            self.0 = std::ptr::null_mut();
        }
    }
}

// Platform-specific FILE* conversion and fclose declaration.
//
// On Windows, std::fs::File wraps an OS HANDLE; we convert via
// _open_osfhandle + _fdopen. On Unix, it wraps an fd; we use fdopen.
// Both transfer ownership: fclose in Drop closes the underlying fd.
//
// `mode` must match how the file was opened: POSIX fdopen rejects a mode
// the fd does not allow. The Windows CRT does not check, and that path
// keeps its fixed "r+b".

unsafe extern "C" {
    fn fclose(stream: *mut ffi::FILE) -> i32;
}

#[cfg(windows)]
fn file_to_cfile(file: std::fs::File, _mode: &std::ffi::CStr) -> Result<*mut ffi::FILE, RfcError> {
    use std::os::windows::io::IntoRawHandle;

    unsafe extern "C" {
        fn _open_osfhandle(osfhandle: isize, flags: i32) -> i32;
        fn _fdopen(fd: i32, mode: *const i8) -> *mut ffi::FILE;
    }

    const O_BINARY: i32 = 0x8000;

    // Transfer ownership from Rust File to OS handle.
    let handle = file.into_raw_handle();

    // Safety: handle is a valid OS file handle from File::create/open.
    // _open_osfhandle takes ownership; the CRT fd must be closed via
    // fclose (or _close), not CloseHandle.
    let fd = unsafe { _open_osfhandle(handle as isize, O_BINARY) };
    if fd == -1 {
        return Err(RfcError::InvalidInput(
            "_open_osfhandle failed: the OS handle could not be associated with a CRT fd".into(),
        ));
    }

    let mode = c"r+b";
    // Safety: fd is a valid CRT file descriptor. _fdopen takes ownership;
    // the stream must be closed via fclose, not _close.
    let ptr = unsafe { _fdopen(fd, mode.as_ptr()) };
    if ptr.is_null() {
        return Err(RfcError::InvalidInput(
            "_fdopen failed: could not open a stdio stream on the CRT fd".into(),
        ));
    }
    Ok(ptr)
}

#[cfg(unix)]
fn file_to_cfile(file: std::fs::File, mode: &std::ffi::CStr) -> Result<*mut ffi::FILE, RfcError> {
    use std::os::unix::io::IntoRawFd;

    unsafe extern "C" {
        // The return type is the platform's FILE*. The SDK receives it as an
        // opaque pointer and passes it to the same platform C library, so
        // the bindgen FILE type is never dereferenced on our side.
        fn fdopen(fd: i32, mode: *const std::ffi::c_char) -> *mut ffi::FILE;
    }

    let fd = file.into_raw_fd();
    // Safety: fd is a valid file descriptor from File::create/open.
    // fdopen takes ownership; fclose will close the underlying fd.
    let ptr = unsafe { fdopen(fd, mode.as_ptr()) };
    if ptr.is_null() {
        return Err(RfcError::InvalidInput(
            "fdopen failed: could not open a stdio stream on the fd".into(),
        ));
    }
    Ok(ptr)
}

// ── Public API ────────────────────────────────────────────────────────

/// Save the SDK's in-memory repository cache to a file.
///
/// The repository is identified by `repo_id` (typically the SAP system
/// ID). The file format is the SDK's own binary format, readable by
/// `load` and by JCo/NCo.
///
/// The cache must have been populated first — typically by calling
/// `Connection::fetch_function_desc` for each function module.
///
/// A `repo_id` with an embedded NUL fails with `RfcError::InvalidInput`
/// before the SDK call (the same rule for `load` and `create_function`).
pub fn save(repo_id: &str, path: &Path) -> Result<(), RfcError> {
    let id_uc = uc::encode_nul_terminated(repo_id, || "repository id".into())?;
    let sdk = ffi::sdk()?;
    let file = CFile::create(path)?;
    let mut error_info = ffi::zero_error_info();
    let cache = ffi::lock_metadata_cache();
    // Safety: id_uc is null-terminated, file is a valid FILE* stream.
    let rc = unsafe { sdk.RfcSaveRepository(id_uc.as_ptr(), file.as_ptr(), &mut error_info) };
    drop(cache);
    drop(file);
    if rc != ffi::RFC_RC::RFC_OK {
        let info = ffi::extract_error_info("RfcSaveRepository", &error_info);
        return Err(RfcError::from_error_info(info));
    }
    Ok(())
}

/// Load a repository file into the SDK's in-memory cache.
///
/// After loading, function descriptors are available via
/// `create_function` without a backend connection. Any previously
/// cached metadata under the same `repo_id` is replaced (SDK rule).
///
/// Trap: the replaced descriptors are freed. Load only while no call,
/// `TableReader` or `FuncDesc` walk of the same `repo_id` (for a live
/// connection: the system ID) is in progress in the process; the metadata
/// lock cannot protect a descriptor a caller already holds.
pub fn load(repo_id: &str, path: &Path) -> Result<(), RfcError> {
    let id_uc = uc::encode_nul_terminated(repo_id, || "repository id".into())?;
    let sdk = ffi::sdk()?;
    let file = CFile::open(path)?;
    let mut error_info = ffi::zero_error_info();
    let cache = ffi::lock_metadata_cache();
    // Safety: id_uc is null-terminated, file is a valid FILE* stream.
    let rc = unsafe { sdk.RfcLoadRepository(id_uc.as_ptr(), file.as_ptr(), &mut error_info) };
    drop(cache);
    drop(file);
    if rc != ffi::RFC_RC::RFC_OK {
        let info = ffi::extract_error_info("RfcLoadRepository", &error_info);
        return Err(RfcError::from_error_info(info));
    }
    Ok(())
}

/// Create a function container from cached metadata.
///
/// The repository must have been loaded first via `load`. The returned
/// `Function` wraps an `RFC_FUNCTION_HANDLE` with RAII cleanup and
/// provides `set_inputs` / `read_outputs` for offline round-trip testing.
pub fn create_function(repo_id: &str, func_name: &str) -> Result<Function, RfcError> {
    let id_uc = uc::encode_nul_terminated(repo_id, || "repository id".into())?;
    let sdk = ffi::sdk()?;
    let name_uc = uc::encode_nul_terminated(func_name, || "function name".into())?;
    let mut error_info = ffi::zero_error_info();

    let cache = ffi::lock_metadata_cache();
    // Safety: id_uc and name_uc are null-terminated. The returned handle
    // is cached by the SDK — not owned by us (do not free).
    let desc =
        unsafe { sdk.RfcGetCachedFunctionDesc(id_uc.as_ptr(), name_uc.as_ptr(), &mut error_info) };
    drop(cache);
    if desc.is_null() {
        let info = ffi::extract_error_info("RfcGetCachedFunctionDesc", &error_info);
        return Err(RfcError::from_error_info(info));
    }

    error_info = ffi::zero_error_info();
    // Safety: desc is a valid function descriptor from the cache.
    let handle = unsafe { sdk.RfcCreateFunction(desc, &mut error_info) };
    if handle.is_null() {
        let info = ffi::extract_error_info("RfcCreateFunction", &error_info);
        return Err(RfcError::from_error_info(info));
    }

    Ok(Function { sdk, desc, handle })
}

/// An offline function container — RAII wrapper around
/// `RFC_FUNCTION_HANDLE` with `set_inputs` / `read_outputs`.
pub struct Function {
    sdk: &'static ffi::SapNwRfc,
    desc: ffi::RFC_FUNCTION_DESC_HANDLE,
    handle: ffi::RFC_FUNCTION_HANDLE,
}

impl Function {
    /// Populate IMPORT/CHANGING/TABLES parameters on the container.
    pub fn set_inputs(
        &self,
        inputs: &IndexMap<String, RfcValue>,
        compat: CompatOptions,
    ) -> Result<(), RfcError> {
        marshal::set_inputs(self.sdk, self.desc, self.handle, inputs, compat)
    }

    /// Read all EXPORT/CHANGING/TABLES parameters from the container.
    pub fn read_outputs(&self) -> Result<IndexMap<String, RfcValue>, RfcError> {
        marshal::read_outputs(self.sdk, self.desc, self.handle, CompatOptions::default())
    }

    /// Read ALL parameters including IMPORT — for offline round-trip
    /// testing where `RfcInvoke` is never called.
    pub fn read_all(&self) -> Result<IndexMap<String, RfcValue>, RfcError> {
        marshal::read_all(self.sdk, self.desc, self.handle)
    }

    /// Hand the container to the chunked reader, as
    /// `Connection::call_chunked` does after `RfcInvoke` — for offline
    /// tests of `TableReader`. `compat` selects the outputs as in
    /// `call_chunked` (`return_import_params` exposes the tables the test
    /// filled).
    pub fn into_chunked_outputs(
        mut self,
        compat: CompatOptions,
    ) -> Result<IndexMap<String, ChunkedOutput>, RfcError> {
        let handle = std::mem::replace(&mut self.handle, std::ptr::null_mut());
        chunked::open_outputs(self.desc, FuncHandle::from_raw(handle), compat)
    }
}

impl Drop for Function {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            let mut error_info = ffi::zero_error_info();
            // Safety: handle was returned by RfcCreateFunction and is
            // non-null. Destroying it releases the container memory.
            unsafe {
                self.sdk.RfcDestroyFunction(self.handle, &mut error_info);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}
