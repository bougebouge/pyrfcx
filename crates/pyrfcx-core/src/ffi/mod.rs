// Runtime SDK loader and FFI bindings.

// One committed bindings file per target. The files are not
// interchangeable: FILE, the DecFloat34 alignment and the C enum signedness
// differ between MSVC x64 and arm64 Darwin, and a generated layout assertion
// checks only the Rust declaration, so the wrong file still compiles.
#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_env = "gnu", target_arch = "aarch64"),
    all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"),
)))]
compile_error!(
    "pyrfcx-core has no FFI bindings for this target. Generate them with \
     `just regen-bindings` on the target and select the file in ffi/mod.rs."
);

#[doc(hidden)]
#[cfg_attr(
    all(target_os = "macos", target_arch = "aarch64"),
    path = "bindings_aarch64_apple_darwin.rs"
)]
#[cfg_attr(
    all(target_os = "linux", target_env = "gnu", target_arch = "aarch64"),
    path = "bindings_aarch64_unknown_linux_gnu.rs"
)]
#[cfg_attr(
    all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"),
    path = "bindings_x86_64_unknown_linux_gnu.rs"
)]
#[allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    dead_code,
    unsafe_op_in_unsafe_fn,
    clippy::all
)]
pub mod bindings;

use std::sync::{
    Mutex, MutexGuard, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard,
};

use crate::error::{RfcError, RfcErrorInfo};
use crate::uc;

pub(crate) use bindings::SapNwRfc;
// Re-export bindgen types needed by other modules.
pub(crate) use bindings::{
    DATA_CONTAINER_HANDLE, FILE, RFC_ATTRIBUTES, RFC_CONNECTION_HANDLE, RFC_CONNECTION_PARAMETER,
    RFC_DIRECTION, RFC_ERROR_INFO, RFC_FIELD_DESC, RFC_FUNCTION_DESC_HANDLE, RFC_FUNCTION_HANDLE,
    RFC_PARAMETER_DESC, RFC_RC, RFC_STRUCTURE_HANDLE, RFC_TABLE_HANDLE, RFC_TYPE_DESC_HANDLE,
    RFCTYPE,
};

#[cfg(target_os = "windows")]
const LIB_NAME: &str = "sapnwrfc.dll";
#[cfg(target_os = "linux")]
const LIB_NAME: &str = "libsapnwrfc.so";
#[cfg(target_os = "macos")]
const LIB_NAME: &str = "libsapnwrfc.dylib";

#[cfg(target_os = "windows")]
unsafe extern "system" {
    fn SetDllDirectoryW(lpPathName: *const u16) -> i32;
    fn AddDllDirectory(NewDirectory: *const u16) -> *mut std::ffi::c_void;
    fn SetDefaultDllDirectories(DirectoryFlags: u32) -> i32;
}

/// `LOAD_LIBRARY_SEARCH_DEFAULT_DIRS` — application dir + `AddDllDirectory`
/// entries + System32. Notably NOT `PATH`.
#[cfg(target_os = "windows")]
const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: u32 = 0x0000_1000;

// Fallback for get_or_try_init (once_cell_try not yet stable):
// store Result<SapNwRfc, RfcError> inside OnceLock.
static SDK: OnceLock<Result<SapNwRfc, RfcError>> = OnceLock::new();

// ── Process-wide SDK state ─────────────────────────────────────────────
//
// The guide allows any API call from any thread at the same time, on
// different handles. Two kinds of SDK state are shared by every handle
// in the process, and pyrfcx guards both itself. No SDK call made under
// one of these locks takes the other, and neither is ever held while
// waiting for the Python GIL.

/// Guards the SDK's metadata (DDIC) cache: every call that can add to,
/// replace or read the cache by name.
static METADATA_CACHE: Mutex<()> = Mutex::new(());

/// Guards the process-wide configuration that `RfcOpenConnection` reads:
/// the `sapnwrfc.ini` contents and the crypto library path.
static SDK_CONFIG: RwLock<()> = RwLock::new(());

/// Hold this across every SDK call that looks up the metadata cache by
/// name or writes it (`RfcGetFunctionDesc`, `RfcGetCachedFunctionDesc`,
/// `RfcSaveRepository`, `RfcLoadRepository`).
///
/// Trap: concurrent cold `RfcGetFunctionDesc` calls, each on its own
/// connection as the guide allows, leave a broken entry in the shared
/// cache. Nothing fails until process exit, where the SDK's static
/// repository destructor reads a null pointer and the process dies with
/// SIGSEGV after `main` returned. pyrfc never shows it because the GIL
/// serializes its lookups. A cached lookup is cheap, so the lock costs
/// nothing measurable; a cold miss is fetched once instead of once per
/// waiting thread.
///
/// Reading a descriptor that was already returned (parameter, field and
/// type descriptions) needs no lock: the guide calls metadata immutable
/// and shareable across threads.
///
/// Public only for raw-FFI callers outside the crate (live probes, the
/// fact collector), which must take it too.
#[doc(hidden)]
pub fn lock_metadata_cache() -> MutexGuard<'static, ()> {
    METADATA_CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Shared side of [`SDK_CONFIG`]: hold it across `RfcOpenConnection`,
/// which reads the ini contents (for `dest`) and the crypto library path.
/// Opens still run in parallel with each other.
pub(crate) fn read_sdk_config() -> RwLockReadGuard<'static, ()> {
    SDK_CONFIG.read().unwrap_or_else(PoisonError::into_inner)
}

/// Exclusive side of [`SDK_CONFIG`]: hold it across `RfcSetIniPath`,
/// `RfcReloadIniFile` and `RfcLoadCryptoLibrary`.
///
/// Trap: these replace process-wide state that a parallel
/// `RfcOpenConnection` is reading. The guide shows them only at startup or
/// between closing and opening connections, never alongside an open. The
/// lock makes a change wait for the opens in progress and keeps new opens
/// out until it is done. A connection that is already open is not
/// affected by either.
fn write_sdk_config() -> RwLockWriteGuard<'static, ()> {
    SDK_CONFIG.write().unwrap_or_else(PoisonError::into_inner)
}

/// Returns a reference to the loaded SDK function table.
///
/// The library is loaded on the first call and reused thereafter.
/// Returns `Err(RfcError::SdkNotFound(...))` if the library cannot
/// be found or any required symbol is missing.
#[doc(hidden)]
pub fn sdk() -> Result<&'static SapNwRfc, RfcError> {
    SDK.get_or_init(load_sdk).as_ref().map_err(|e| e.clone())
}

/// Attempt to load the SDK library. Called at most once by `OnceLock`.
fn load_sdk() -> Result<SapNwRfc, RfcError> {
    let home = std::env::var("SAPNWRFC_HOME").ok();
    let (lib_path, home_display) = match &home {
        Some(h) => {
            let mut path = std::path::PathBuf::from(h);
            path.push("lib");
            path.push(LIB_NAME);
            let display = h.clone();
            (path.to_string_lossy().into_owned(), display)
        }
        None => (LIB_NAME.to_string(), "not set".to_string()),
    };

    // On Windows, make the SDK's lib dir searchable so its sibling DLLs
    // (ICU, libsapucum) can be found.
    //
    // This must PERSIST for the life of the process, not just across the
    // load below: sapnwrfc.dll has no static dependency on ICU (checked with
    // `dumpbin -dependents`: only Windows system DLLs) — it LoadLibrary's
    // icu*57.dll lazily, at first use, long after this function returns. An
    // earlier version set the directory only around `load_library` and then
    // reset it, which made `RfcOpenConnection` fail with a MODAL "could not
    // open ICU library" dialog — a hang, not an error, in any non-interactive
    // run. It went unnoticed only because an ambient C:\Windows\System32
    // sapnwrfc.dll was being loaded instead of the vendored one, and
    // System32 holds ICU too.
    //
    // Two mechanisms, because which one works depends on the host process:
    //   * SetDllDirectoryW — honoured by the classic search order (a plain
    //     Rust process), but IGNORED once SetDefaultDllDirectories has been
    //     called, which CPython 3.8+ does at startup.
    //   * AddDllDirectory — the modern, additive equivalent, honoured only
    //     when the default dirs are in force; hence SetDefaultDllDirectories
    //     first (a no-op in a Python host, which already called it).
    // Both are additive and neither is undone.
    #[cfg(target_os = "windows")]
    if let Some(h) = &home {
        let mut dir = std::path::PathBuf::from(h);
        dir.push("lib");
        let wide: Vec<u16> = dir
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // Safety: all three take a valid null-terminated wide string or a
        // plain flag word; `wide` outlives the calls, which copy it.
        unsafe {
            SetDllDirectoryW(wide.as_ptr());
            SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
            AddDllDirectory(wide.as_ptr());
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if let Some(h) = &home {
        let mut dir = std::path::PathBuf::from(h);
        dir.push("lib");
        preload_icu(&dir)?;
    }

    let result = load_library(&lib_path);

    match result {
        Ok(sdk) => Ok(sdk),
        Err(e) => Err(RfcError::SdkNotFound(format!(
            "Failed to load SAP NW RFC SDK library.\n\
             Library: {LIB_NAME}\n\
             Path tried: {lib_path}\n\
             SAPNWRFC_HOME: {home_display}\n\
             OS error: {e}\n\
             Set SAPNWRFC_HOME to the SDK root directory."
        ))),
    }
}

#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;

/// Open the SDK's ICU libraries from `lib_dir` and keep them loaded.
///
/// The SDK opens ICU lazily, at the first connection, by leaf name. dyld
/// resolves a leaf name through `DYLD_LIBRARY_PATH` and the SDK's own
/// LC_RPATH — an absolute install path fixed when SAP built the library —
/// and never through `SAPNWRFC_HOME`. So an SDK outside that install path
/// silently takes ICU from whatever SDK sits there, or, when none does, the
/// SDK terminates the process. A leaf-name lookup returns an image that is
/// already loaded, so opening ICU here by full path pins it to `lib_dir`.
///
/// Linux: the SDK tries the leaf name first — `LD_LIBRARY_PATH`, its
/// RUNPATH (SAP install paths), the system directories — and only then its
/// own directory. So an ICU on that search path silently wins over the one
/// in `lib_dir`. glibc matches a leaf name against the SONAME of an image
/// that is already loaded, so the same preload pins it.
///
/// Dependency order (data, common, i18n): each later library links the
/// earlier ones by install name. No ICU file in `lib_dir` is not an error;
/// the SDK then does its own search.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn preload_icu(lib_dir: &std::path::Path) -> Result<(), RfcError> {
    use libloading::os::unix::{Library, RTLD_GLOBAL, RTLD_NOW};

    let Ok(entries) = std::fs::read_dir(lib_dir) else {
        return Ok(());
    };
    let names: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();

    // The version suffix differs by patch level (`libicuuc.50.dylib`,
    // `libicuuc57.dylib`), so match on the stem only.
    let suffix = if cfg!(target_os = "macos") {
        ".dylib"
    } else {
        ".so"
    };
    for prefix in ["libicudata", "libicuuc", "libicui18n"] {
        let Some(name) = names
            .iter()
            .find(|n| n.starts_with(prefix) && n.ends_with(suffix))
        else {
            continue;
        };
        let path = lib_dir.join(name);
        // Safety: an ICU library runs no initialiser that depends on the
        // caller. The handle is released with `into_raw` and never closed,
        // so the image stays loaded for the life of the process, as the SDK
        // expects.
        let lib = unsafe { Library::open(Some(&path), RTLD_NOW | RTLD_GLOBAL) }.map_err(|e| {
            RfcError::SdkNotFound(format!(
                "Failed to load the SAP NW RFC SDK's ICU library.\n\
                 Path tried: {}\n\
                 OS error: {e}",
                path.display()
            ))
        })?;
        let _ = lib.into_raw();
    }
    Ok(())
}

fn load_library(path: &str) -> Result<SapNwRfc, libloading::Error> {
    // Safety: SapNwRfc::new loads the shared library from `path` and resolves
    // all required symbols (--dynamic-link-require-all). The library remains
    // loaded for the process lifetime (stored in OnceLock).
    unsafe { SapNwRfc::new(path) }
}

/// Create a zeroed `RFC_ERROR_INFO` for use as an output buffer.
#[doc(hidden)]
pub fn zero_error_info() -> RFC_ERROR_INFO {
    // Safety: RFC_ERROR_INFO is a plain C struct of integers and u16 arrays;
    // all-zero is a valid representation (code=0, group=0, all strings empty).
    unsafe { std::mem::zeroed() }
}

/// Convert a raw `RFC_ERROR_INFO` from the SDK to our decoded `RfcErrorInfo`.
///
/// `sdk_function` names the SDK function that filled `raw`.
#[doc(hidden)]
// The bindgen enum newtypes wrap c_int on MSVC and c_uint elsewhere.
#[allow(clippy::unnecessary_cast)]
pub fn extract_error_info(sdk_function: &'static str, raw: &RFC_ERROR_INFO) -> RfcErrorInfo {
    RfcErrorInfo {
        sdk_function,
        code: raw.code.0 as u32,
        group: raw.group.0 as u32,
        key: uc::decode(&raw.key),
        message: uc::decode(&raw.message),
        abap_msg_class: uc::decode(&raw.abapMsgClass),
        abap_msg_type: uc::decode(&raw.abapMsgType),
        abap_msg_number: uc::decode_numc(&raw.abapMsgNumber),
        abap_msg_v1: uc::decode(&raw.abapMsgV1),
        abap_msg_v2: uc::decode(&raw.abapMsgV2),
        abap_msg_v3: uc::decode(&raw.abapMsgV3),
        abap_msg_v4: uc::decode(&raw.abapMsgV4),
    }
}

/// Load the SDK and call `RfcGetVersion`, returning (major, minor, patch_level, description).
pub fn get_version() -> Result<(u32, u32, u32, String), RfcError> {
    let sdk = sdk()?;
    let mut major: std::ffi::c_uint = 0;
    let mut minor: std::ffi::c_uint = 0;
    let mut patch: std::ffi::c_uint = 0;
    // Safety: RfcGetVersion writes to the three out-params and returns a
    // pointer to a static UTF-16 string. The pointer is valid for the
    // process lifetime (SDK docs: "static memory, do NOT free").
    let desc_ptr = unsafe { (sdk.RfcGetVersion)(&mut major, &mut minor, &mut patch) };
    let description = if desc_ptr.is_null() {
        String::new()
    } else {
        // Safety: the SDK guarantees a null-terminated UTF-16 string in
        // static memory. We scan for the null terminator.
        let slice = unsafe {
            let mut len = 0usize;
            while *desc_ptr.add(len) != 0 {
                len += 1;
            }
            std::slice::from_raw_parts(desc_ptr, len)
        };
        String::from_utf16_lossy(slice)
    };
    Ok((major, minor, patch, description))
}

/// Encode `ConnectionParams` into the SDK's `RFC_CONNECTION_PARAMETER` array.
///
/// Returns `(params, buffers)`. The `params` array borrows from `buffers` —
/// both must live until `RfcOpenConnection` returns.
///
/// A name or value with an embedded NUL fails with
/// [`RfcError::InvalidInput`]. The message names the parameter but never
/// shows the value, which can be a password.
pub(crate) fn encode_connection_params(
    params: &crate::connection::ConnectionParams,
) -> Result<(Vec<RFC_CONNECTION_PARAMETER>, Vec<Vec<u16>>), RfcError> {
    let count = params.len();
    // Two UTF-16 buffers per entry (name + value).
    let mut buffers: Vec<Vec<u16>> = Vec::with_capacity(count * 2);
    let mut rfc_params: Vec<RFC_CONNECTION_PARAMETER> = Vec::with_capacity(count);

    for (name, value) in params.iter() {
        buffers.push(uc::encode_nul_terminated(name, || {
            "connection parameter name".into()
        })?);
        buffers.push(uc::encode_nul_terminated(value, || {
            format!("connection parameter '{name}'")
        })?);
    }

    for i in 0..count {
        rfc_params.push(RFC_CONNECTION_PARAMETER {
            name: buffers[i * 2].as_ptr(),
            value: buffers[i * 2 + 1].as_ptr(),
        });
    }

    Ok((rfc_params, buffers))
}

// Expose load_sdk's inner logic for testing without the OnceLock.
// Integration tests call this to verify failure paths with different
// env var states in the same process.
#[doc(hidden)]
pub fn try_load_sdk() -> Result<(), RfcError> {
    load_sdk().map(|_| ())
}

/// Set the directory where the SDK looks for `sapnwrfc.ini`
/// (`RfcSetIniPath`). The SDK reads the file from the new directory at once.
///
/// Process-wide: the setting applies to every connection opened afterwards.
/// Traps:
/// - Destinations the SDK has already read stay known after a switch to
///   another directory.
/// - Setting the directory the SDK already has does not read the file
///   again; use [`reload_ini_file`] after an edit.
/// - The SDK accepts a directory that does not exist.
///
/// A path with an embedded NUL fails with [`RfcError::InvalidInput`]
/// before any SDK call.
pub fn set_ini_path(dir: &str) -> Result<(), RfcError> {
    let dir_uc = uc::encode_nul_terminated(dir, || "path".into())?;
    let sdk = sdk()?;
    let mut error_info = zero_error_info();
    let _config = write_sdk_config();
    // Safety: dir_uc is a valid null-terminated UTF-16 path.
    let rc = unsafe { sdk.RfcSetIniPath(dir_uc.as_ptr(), &mut error_info) };
    if rc != RFC_RC::RFC_OK {
        let info = extract_error_info("RfcSetIniPath", &error_info);
        return Err(RfcError::from_error_info(info));
    }
    Ok(())
}

/// Read `sapnwrfc.ini` again (`RfcReloadIniFile`), from the directory set
/// by [`set_ini_path`] or the current working directory.
///
/// Process-wide. Destinations the SDK read earlier stay known, even when
/// the new file no longer defines them. A missing file is not an error:
/// the SDK then uses its defaults.
pub fn reload_ini_file() -> Result<(), RfcError> {
    let sdk = sdk()?;
    let mut error_info = zero_error_info();
    let _config = write_sdk_config();
    // Safety: error_info is a valid, writable RFC_ERROR_INFO.
    let rc = unsafe { sdk.RfcReloadIniFile(&mut error_info) };
    if rc != RFC_RC::RFC_OK {
        let info = extract_error_info("RfcReloadIniFile", &error_info);
        return Err(RfcError::from_error_info(info));
    }
    Ok(())
}

/// Set the path of the SAP crypto library for TLS over WebSocket RFC
/// (`RfcLoadCryptoLibrary`; same effect as `TLS_SAPCRYPTOLIB` in
/// `sapnwrfc.ini`). The path includes the file name. SNC does not use this
/// setting: SNC takes its library from `SNC_LIB`.
///
/// Process-wide and final after the first call: the SDK keeps the first
/// path it is given, even when that library fails to load. A later call
/// with the same path succeeds; a call with another path fails. An empty
/// path is rejected and sets nothing.
///
/// A path with an embedded NUL fails with [`RfcError::InvalidInput`]
/// before any SDK call, so it does not fix the setting.
pub fn load_crypto_library(path: &str) -> Result<(), RfcError> {
    let path_uc = uc::encode_nul_terminated(path, || "path".into())?;
    let sdk = sdk()?;
    let mut error_info = zero_error_info();
    let _config = write_sdk_config();
    // Safety: path_uc is a valid null-terminated UTF-16 path.
    let rc = unsafe { sdk.RfcLoadCryptoLibrary(path_uc.as_ptr(), &mut error_info) };
    if rc != RFC_RC::RFC_OK {
        let info = extract_error_info("RfcLoadCryptoLibrary", &error_info);
        return Err(RfcError::from_error_info(info));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_nul_in_a_path_is_rejected_before_the_sdk() {
        for result in [set_ini_path("a\0b"), load_crypto_library("a\0b")] {
            match result {
                Err(RfcError::InvalidInput(msg)) => {
                    assert_eq!(msg, "embedded null character in path")
                }
                other => panic!("expected InvalidInput, got {other:?}"),
            }
        }
    }
}
