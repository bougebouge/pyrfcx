// Connection lifecycle: params, open, close, call, cancel, Drop.

use std::sync::{Arc, Mutex};

use indexmap::IndexMap;

use crate::chunked::{self, ChunkedOutput};
use crate::describe;
use crate::error::{RfcError, RfcErrorInfo};
use crate::ffi;
use crate::marshal;
use crate::value::RfcValue;

/// Attributes of an open connection, as reported by `RfcGetConnectionAttributes`.
///
/// One field per member of the SDK's `RFC_ATTRIBUTES` (7.50 header, sapnwrfc.h
/// line 230), in header order, with two deliberate departures:
///
/// - `reserved` is not exposed. The header says "reserved for later use"; it
///   carries no defined meaning to hand a caller.
/// - Names are the SDK's own, transliterated to snake_case. The SDK's
///   convention is that an unprefixed field describes *our* side and a
///   `partner*` field the other side (`host`/`partnerHost`, `rel`/`partnerRel`,
///   `codepage`/`partnerCodepage`) — that pairing is preserved, which is why
///   `type` stays `r#type` rather than being renamed to something like
///   `system_type` that would break the parallel.
///
/// Every field is a `String` because every member of `RFC_ATTRIBUTES` is a
/// `SAP_UC` buffer — the SDK reports even numeric-looking values (`trace`,
/// `partner_bytes_per_char`) as text, and the core does not editorialize.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionAttributes {
    /// RFC destination.
    pub dest: String,
    /// Own host name.
    pub host: String,
    /// Partner host name.
    pub partner_host: String,
    /// R/3 system number.
    pub sys_number: String,
    /// R/3 system ID.
    pub sys_id: String,
    /// Client ("Mandant").
    pub client: String,
    /// User.
    pub user: String,
    /// Language.
    pub language: String,
    /// Trace level (0-3).
    pub trace: String,
    /// 2-byte ISO-Language.
    pub iso_language: String,
    /// Own code page.
    pub codepage: String,
    /// Partner code page.
    pub partner_codepage: String,
    /// C/S: RFC Client / RFC Server.
    pub rfc_role: String,
    /// 2/3/E/R: R/2, R/3, Ext, Reg.Ext.
    pub r#type: String,
    /// 2/3/E/R: R/2, R/3, Ext, Reg.Ext.
    pub partner_type: String,
    /// My system release.
    pub rel: String,
    /// Partner system release.
    pub partner_rel: String,
    /// Partner kernel release.
    pub kernel_rel: String,
    /// CPI-C conversation ID.
    pub cpic_conv_id: String,
    /// Name of the calling ABAP program (report, module pool).
    pub prog_name: String,
    /// Number of bytes per character in the backend's current codepage.
    /// Note this is different from the semantics of the PCS parameter.
    pub partner_bytes_per_char: String,
    /// Partner system code page.
    pub partner_system_codepage: String,
    /// Partner IP.
    pub partner_ip: String,
    /// Partner IPv6.
    pub partner_ipv6: String,
}

impl ConnectionAttributes {
    /// Decode every `SAP_UC` buffer of an `RFC_ATTRIBUTES` into owned strings.
    ///
    /// Each member is declared `SAP_UC name[N+1]` — N characters plus a slot
    /// for the terminator — so `uc::decode`'s null-stop is the correct read.
    ///
    /// Trailing U+0020 is then stripped, following the rule for native CHAR
    /// output (the pad char only, never `trim_end`). These are fixed-width
    /// buffers holding blank-padded ABAP CHAR values, and the SDK pads them
    /// inconsistently — live against 750 PL 18, `partner_rel` came back
    /// `"750 "` and `kernel_rel` `"754 "` (padded to their 4-char width) while
    /// `rel` came back `"754"` (unpadded). The trailing blank is an artifact of
    /// fixed-width storage, carries no information, and leaving it in would
    /// make `attrs.partner_rel == "750"` fail for reasons no caller can guess.
    fn from_ffi(a: &ffi::RFC_ATTRIBUTES) -> Self {
        // Strip the pad char only — U+0020 — never `trim_end()`.
        let d = |buf: &[u16]| {
            crate::uc::decode(buf)
                .trim_end_matches('\u{0020}')
                .to_owned()
        };
        Self {
            dest: d(&a.dest),
            host: d(&a.host),
            partner_host: d(&a.partnerHost),
            sys_number: d(&a.sysNumber),
            sys_id: d(&a.sysId),
            client: d(&a.client),
            user: d(&a.user),
            language: d(&a.language),
            trace: d(&a.trace),
            iso_language: d(&a.isoLanguage),
            codepage: d(&a.codepage),
            partner_codepage: d(&a.partnerCodepage),
            rfc_role: d(&a.rfcRole),
            r#type: d(&a.type_),
            partner_type: d(&a.partnerType),
            rel: d(&a.rel),
            partner_rel: d(&a.partnerRel),
            kernel_rel: d(&a.kernelRel),
            cpic_conv_id: d(&a.cpicConvId),
            prog_name: d(&a.progName),
            partner_bytes_per_char: d(&a.partnerBytesPerChar),
            partner_system_codepage: d(&a.partnerSystemCodepage),
            partner_ip: d(&a.partnerIP),
            partner_ipv6: d(&a.partnerIPv6),
        }
    }
}

/// Connection parameters as untyped key-value pairs.
///
/// Builder-style: `ConnectionParams::new().set("ashost", "sap01").set("client", "100")`.
/// No validation — the SDK validates on `RfcOpenConnection`.
#[derive(Debug, Clone, Default)]
pub struct ConnectionParams {
    entries: Vec<(String, String)>,
}

impl ConnectionParams {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.entries.push((name.into(), value.into()));
        self
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// Legacy-compatibility switches for `call` marshalling — NOT an SDK
/// concept. The SAP NW RFC SDK has no per-call option structure of any
/// kind. This struct exists solely for pyrfc retrocompatibility. Native callers always use `CompatOptions::default()`
/// (all flags false).
#[derive(Debug, Clone, Copy, Default)]
pub struct CompatOptions {
    /// When true, empty-string inputs targeting DATE or TIME fields are
    /// skipped (no setter call). In the native default (this flag and
    /// `pyrfc_date_time_setters` both false), the empty string is sent as
    /// the type's initial value (`00000000` / `000000`), so every SDK patch
    /// level gives the same result.
    pub skip_empty_date_time_inputs: bool,
    /// When true, the fixed-width length check is skipped: over-long values for
    /// fixed-length targets (CHAR, NUMC, BYTE) reach the SDK unchecked
    /// and the SDK truncates (ABAP MOVE semantics — the pyrfc behavior).
    /// When false (native default), over-long input is rejected with
    /// `RfcError::InvalidInput` before any setter call. NOT an SDK
    /// concept; exists solely for pyrfc retrocompatibility.
    pub skip_input_length_check: bool,
    /// When true, integer inputs use pyrfc 3.4's setter dispatch:
    /// INT/INT1/INT2 targets all go through `RfcSetInt` whenever the
    /// value fits i32 (so field-range overflow like 256→INT1 reaches
    /// the SDK, which wraps it silently, as with pyrfc), INT8 through
    /// `RfcSetInt8` when it fits i64. When false (native default), the
    /// value is range-checked against the target width and set with the
    /// width-correct typed setter; violations raise
    /// `RfcError::InvalidInput` before any setter call. NOT an SDK
    /// concept; exists solely for pyrfc retrocompatibility.
    pub pyrfc_int_setters: bool,
    /// When true, the decimal-input check is skipped: a non-numeric
    /// string targeting a BCD/DECF16/DECF34 field reaches `RfcSetString`
    /// unchecked. When false (native default), such input is rejected with
    /// `RfcError::InvalidInput` before any setter call, because the SDK
    /// **silently garbage-converts** it (`"A"` → `1.00`) rather than erroring.
    /// Compat sets this true because the pyrfc-exact client-side validation
    /// runs earlier in the pyrfcx-py pre-pass — the core check would
    /// be redundant there and might diverge from pyrfc's `Decimal()` grammar.
    /// NOT an SDK concept; exists solely for pyrfc retrocompatibility.
    pub skip_decimal_input_check: bool,
    /// When true, a parameter name passed as inactive is NOT looked up
    /// before `RfcSetParameterActive`: the SDK alone decides. pyrfc 3.4 does
    /// no lookup of its own, so below SDK 7.50 PL 15 (where
    /// `RfcSetParameterActive` returns `RFC_OK` for an unknown name) an
    /// unknown name is silently ignored, and from PL 15 the SDK raises.
    /// When false (native default), an unknown name raises on every patch
    /// level. NOT an SDK concept; exists solely for pyrfc retrocompatibility.
    pub skip_inactive_name_check: bool,
    /// When true, inactive parameters are deactivated BEFORE the inputs are
    /// set (pyrfc 3.4's order in `Connection.call`). The SDK then decides
    /// what a value for an inactive parameter does: a setter on the
    /// function handle re-activates a scalar parameter and sends it, while
    /// a structure or table filled through its own handle stays inactive.
    /// When false (native default), deactivation runs after all setters, so
    /// it always wins. NOT an SDK concept; exists solely for pyrfc
    /// retrocompatibility.
    pub deactivate_before_inputs: bool,
    /// When true, output parameters are read back even when inactive: pyrfc
    /// 3.4 never asks `RfcIsParameterActive`, so it returns an inactive
    /// output with whatever the container holds (initial value, or the
    /// caller's own rows for a table). When false (native default), an
    /// inactive output is omitted from the result. NOT an SDK
    /// concept; exists solely for pyrfc retrocompatibility.
    pub read_inactive_outputs: bool,
    /// When true, string input for DATE/TIME fields goes through the SDK's
    /// fixed-width `RfcSetDate`/`RfcSetTime` (pyrfc 3.4's setters): an
    /// over-long value is cut to 8/6 characters and a short one is padded
    /// with NUL. Only visible when the caller turns pyrfc's `check_date` /
    /// `check_time` off; otherwise the length is already exact. When false
    /// (native default), `RfcSetString` is used and the SDK rejects a
    /// wrong-length value. NOT an SDK concept; exists solely for pyrfc
    /// retrocompatibility.
    pub pyrfc_date_time_setters: bool,
    /// When true, IMPORT-direction parameters are read back after the call
    /// and returned with the outputs (pyrfc 3.4's `return_import_params`
    /// config), in descriptor order. The value read back is what the
    /// container holds: the caller's input as the SDK stored it, or the
    /// initial value when no input was set. When false (native default),
    /// IMPORT parameters are never returned. NOT an SDK concept; exists
    /// solely for pyrfc retrocompatibility.
    pub return_import_params: bool,
}

/// Shared cell holding the raw connection handle, so a [`CancelToken`] can
/// reach it from another thread while the owning thread is blocked inside
/// `RfcInvoke` (`RfcCancel` is the SDK's one cross-thread operation).
///
/// The mutex serializes `RfcCancel` against `RfcCloseConnection`: the handle
/// is only ever freed (`close`/`Drop`) while holding the lock, so a token
/// can never hand the SDK a freed handle. The lock is never held across a
/// blocking SDK call — `call` copies the handle out first — so a cancel is
/// never made to wait for the very call it is trying to cancel.
struct HandleCell(Mutex<ffi::RFC_CONNECTION_HANDLE>);

// Safety: RFC_CONNECTION_HANDLE is a pointer to an opaque SDK-internal
// struct. The SDK documents that a connection may be used from any thread
// (one call at a time — enforced by the Mutex in pyrfcx-py), and RfcCancel
// is explicitly specified to be called from a different thread than the one
// executing the call. All shared access goes through the Mutex.
unsafe impl Send for HandleCell {}
unsafe impl Sync for HandleCell {}

impl HandleCell {
    fn get(&self) -> ffi::RFC_CONNECTION_HANDLE {
        *self.0.lock().expect("connection handle mutex poisoned")
    }

    /// Null the handle WITHOUT calling `RfcCloseConnection`.
    ///
    /// Used when the SDK has already closed the connection as part of an
    /// error (guide Table 4-A). The SDK frees the handle at that point and
    /// reuses the address for new connections immediately, so passing it
    /// to ANY further SDK call — `RfcCloseConnection` included — is a
    /// use-after-free that can close an unrelated live connection
    /// (probed: `tests/live_dead_handle.rs`; this was the root cause of
    /// an intermittent test failure).
    fn abandon(&self) {
        *self.0.lock().expect("connection handle mutex poisoned") = std::ptr::null_mut();
    }
}

/// An open connection to a SAP system.
///
/// Wraps an `RFC_CONNECTION_HANDLE`. The handle is closed on `Drop`.
/// Does not track alive/dead state — the SDK returns
/// `RFC_INVALID_HANDLE` on dead handles.
///
/// `Send` but deliberately not `Sync`: the SDK allows a connection to be
/// used from any thread but only one call at a time — the `PhantomData`
/// marker (`Cell<()>` is `Send + !Sync`) keeps the auto-`Sync` that
/// `Arc<HandleCell>` would otherwise grant from re-appearing. Cross-thread
/// cancellation goes through [`CancelToken`], which is `Sync` because
/// `RfcCancel` is the SDK's one sanctioned cross-thread operation.
pub struct Connection {
    cell: Arc<HandleCell>,
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

/// Cancels an in-flight RFC call from another thread (`RfcCancel`).
///
/// Obtained from [`Connection::cancel_token`]; cheap to clone, `Send` +
/// `Sync`. The token shares the connection's handle cell, so it stays safe
/// (and simply errors) after the connection is closed or dropped — it never
/// keeps the connection alive and never frees the handle itself.
#[derive(Clone)]
pub struct CancelToken {
    cell: Arc<HandleCell>,
}

impl CancelToken {
    /// Cancel the RFC call currently executing on the connection
    /// (`RfcCancel`). Per the SDK, this also closes the connection.
    ///
    /// Must run on a different thread than the one blocked in the call —
    /// the SDK specifies that, and the blocked thread cannot call anything
    /// anyway. If no call is in flight, the SDK fails with
    /// `RFC_ILLEGAL_STATE`; a closed/dropped connection errors with
    /// "connection is closed".
    pub fn cancel(&self) -> Result<(), RfcError> {
        let sdk = ffi::sdk()?;
        // Hold the lock across RfcCancel: close()/Drop free the handle
        // under this same lock, so the SDK never sees a freed handle here.
        let guard = self
            .cell
            .0
            .lock()
            .expect("connection handle mutex poisoned");
        if guard.is_null() {
            return Err(RfcError::Other(RfcErrorInfo {
                message: "connection is closed".into(),
                ..Default::default()
            }));
        }
        let mut error_info = ffi::zero_error_info();
        // Safety: *guard is a valid connection handle — non-null, and not
        // freed while we hold the lock. error_info is a valid out-param.
        let rc = unsafe { sdk.RfcCancel(*guard, &mut error_info) };
        if rc != ffi::RFC_RC::RFC_OK {
            let info = ffi::extract_error_info("RfcCancel", &error_info);
            return Err(RfcError::from_error_info(info));
        }
        Ok(())
    }
}

impl Connection {
    /// Open a connection with the given parameters.
    ///
    /// Calls `RfcOpenConnection`. Fails with `CommunicationFailure` for
    /// unreachable hosts, `LogonFailure` for bad credentials / SNC, etc.
    ///
    /// A parameter name or value with an embedded NUL fails with
    /// [`RfcError::InvalidInput`] before any SDK call: the SDK would read it
    /// only up to the NUL. The message names the parameter, never its value.
    pub fn open(params: &ConnectionParams) -> Result<Self, RfcError> {
        let (rfc_params, _buffers) = ffi::encode_connection_params(params)?;
        let sdk = ffi::sdk()?;

        let mut error_info = ffi::zero_error_info();

        // The open reads process-wide configuration (ini contents, crypto
        // library path); `set_ini_path` and its siblings must not replace
        // it meanwhile. Opens still run in parallel with each other.
        let _config = ffi::read_sdk_config();

        // Safety: rfc_params points to a valid array of RFC_CONNECTION_PARAMETER
        // structs whose name/value pointers reference the UTF-16 buffers in
        // `_buffers` (alive for the duration of this call). error_info is a
        // valid out-param. The returned handle is null on failure.
        let handle = unsafe {
            sdk.RfcOpenConnection(
                rfc_params.as_ptr(),
                rfc_params.len() as std::ffi::c_uint,
                &mut error_info,
            )
        };

        if handle.is_null() {
            let info = ffi::extract_error_info("RfcOpenConnection", &error_info);
            return Err(RfcError::from_error_info(info));
        }

        Ok(Connection {
            cell: Arc::new(HandleCell(Mutex::new(handle))),
            _not_sync: std::marker::PhantomData,
        })
    }

    /// Close the connection explicitly.
    ///
    /// After this call the handle is null and any further SDK calls on it
    /// will fail with `RFC_INVALID_HANDLE`. Calling `close` on an already-
    /// closed connection is a no-op.
    pub fn close(&mut self) -> Result<(), RfcError> {
        let sdk = ffi::sdk()?;
        // Hold the lock across RfcCloseConnection so a concurrent
        // CancelToken::cancel can never reach a freed handle.
        let mut guard = self
            .cell
            .0
            .lock()
            .expect("connection handle mutex poisoned");
        if guard.is_null() {
            return Ok(());
        }

        let mut error_info = ffi::zero_error_info();

        // Safety: *guard is non-null and was returned by RfcOpenConnection.
        // After this call the handle is invalid; we null it immediately.
        let rc = unsafe { sdk.RfcCloseConnection(*guard, &mut error_info) };

        *guard = std::ptr::null_mut();

        if rc != ffi::RFC_RC::RFC_OK {
            let info = ffi::extract_error_info("RfcCloseConnection", &error_info);
            return Err(RfcError::from_error_info(info));
        }

        Ok(())
    }

    /// A token for cancelling an in-flight `call` from another thread.
    pub fn cancel_token(&self) -> CancelToken {
        CancelToken {
            cell: Arc::clone(&self.cell),
        }
    }

    /// Abandon the handle if the error means the SDK already closed (and
    /// freed) the connection — see `HandleCell::abandon`. Passes the
    /// result through so it can wrap any SDK-touching path.
    fn abandon_if_fatal<T>(&self, result: Result<T, RfcError>) -> Result<T, RfcError> {
        if let Err(e) = &result
            && e.sdk_closed_connection()
        {
            self.cell.abandon();
        }
        result
    }

    #[doc(hidden)]
    pub fn handle(&self) -> ffi::RFC_CONNECTION_HANDLE {
        self.cell.get()
    }

    /// Fetch metadata for a function module from the backend, populating
    /// the SDK's repository cache. Does not invoke the function.
    pub fn fetch_function_desc(&self, name: &str) -> Result<(), RfcError> {
        let _ =
            self.abandon_if_fatal(self.get_function_desc(ffi::sdk()?, self.cell.get(), name))?;
        Ok(())
    }

    /// Describe a function module's signature: every parameter with its
    /// type, direction and — for structures and tables — its nested
    /// fields. Does not invoke the function.
    ///
    /// The name goes to the SDK unchanged, and the SDK lookup is
    /// case-sensitive: a lower-case name fails with "function not found"
    /// (`FU_NOT_FOUND`). This method does not upper-case the name. A name
    /// with an embedded NUL fails with [`RfcError::InvalidInput`] before
    /// the SDK lookup.
    ///
    /// The descriptor itself is cached by the SDK after the first fetch,
    /// so repeated calls cost only the metadata walk, not a round-trip.
    pub fn describe_function(&self, name: &str) -> Result<describe::FuncDesc, RfcError> {
        let handle = self.cell.get();
        if handle.is_null() {
            return Err(RfcError::Other(RfcErrorInfo {
                message: "connection is closed".into(),
                ..Default::default()
            }));
        }
        let sdk = ffi::sdk()?;
        let func_desc = self.abandon_if_fatal(self.get_function_desc(sdk, handle, name))?;
        describe::describe(sdk, func_desc)
    }

    /// Read every attribute of this connection (`RfcGetConnectionAttributes`).
    ///
    /// Errors if the connection is closed — the SDK reports "connection
    /// already closed" via `errorInfo`, and we pre-check the null handle to
    /// avoid handing the SDK a dangling one.
    pub fn attributes(&self) -> Result<ConnectionAttributes, RfcError> {
        let handle = self.cell.get();
        if handle.is_null() {
            return Err(RfcError::Other(RfcErrorInfo {
                message: "connection is closed".into(),
                ..Default::default()
            }));
        }
        let sdk = ffi::sdk()?;
        let mut attrs: ffi::RFC_ATTRIBUTES = unsafe { std::mem::zeroed() };
        let mut error_info = ffi::zero_error_info();
        // Safety: handle is a valid connection handle, attrs is a
        // valid out-param buffer.
        let rc = unsafe { sdk.RfcGetConnectionAttributes(handle, &mut attrs, &mut error_info) };
        if rc != ffi::RFC_RC::RFC_OK {
            let info = ffi::extract_error_info("RfcGetConnectionAttributes", &error_info);
            return self.abandon_if_fatal(Err(RfcError::from_error_info(info)));
        }
        Ok(ConnectionAttributes::from_ffi(&attrs))
    }

    /// Ping the communication partner (`RfcPing`).
    ///
    /// A network round-trip that verifies the connection is genuinely alive —
    /// the SDK's cheaper `RfcIsConnectionHandleValid` only checks its own
    /// bookkeeping and cannot see a connection a firewall silently dropped
    /// (per its header doc, which points at `RfcPing` for that).
    ///
    /// Errors with the SDK's failure if the connection is broken; the null
    /// handle of an explicitly closed connection is pre-checked like
    /// `attributes`.
    pub fn ping(&self) -> Result<(), RfcError> {
        let handle = self.cell.get();
        if handle.is_null() {
            return Err(RfcError::Other(RfcErrorInfo {
                message: "connection is closed".into(),
                ..Default::default()
            }));
        }
        let sdk = ffi::sdk()?;
        let mut error_info = ffi::zero_error_info();
        // Safety: handle is a valid connection handle, error_info is a
        // valid out-param.
        let rc = unsafe { sdk.RfcPing(handle, &mut error_info) };
        if rc != ffi::RFC_RC::RFC_OK {
            let info = ffi::extract_error_info("RfcPing", &error_info);
            return self.abandon_if_fatal(Err(RfcError::from_error_info(info)));
        }
        Ok(())
    }

    /// Whether the SDK still holds this connection in its connection
    /// management (`RfcIsConnectionHandleValid`).
    ///
    /// This asks the SDK about the handle. The Python layer's `closed` is a
    /// different fact: what pyrfcx itself knows about the connection.
    ///
    /// No network round-trip: the SDK checks its own bookkeeping only, so a
    /// connection a firewall silently dropped still reports `true` — `ping`
    /// is the check for that. A closed connection reports `false` without
    /// an SDK call, and so does a handle the SDK rejects: the header
    /// documents the error as "details in case the connection is invalid".
    pub fn is_valid(&self) -> Result<bool, RfcError> {
        // Hold the lock across the SDK call: close()/Drop free the handle
        // under this same lock.
        let guard = self
            .cell
            .0
            .lock()
            .expect("connection handle mutex poisoned");
        if guard.is_null() {
            return Ok(false);
        }
        let sdk = ffi::sdk()?;
        let mut is_valid: std::ffi::c_int = 0;
        let mut error_info = ffi::zero_error_info();
        // Safety: *guard is a non-null connection handle, not freed while
        // we hold the lock; is_valid and error_info are valid out-params.
        let rc = unsafe { sdk.RfcIsConnectionHandleValid(*guard, &mut is_valid, &mut error_info) };
        Ok(rc == ffi::RFC_RC::RFC_OK && is_valid != 0)
    }

    /// Invoke an RFC function module and return its output parameters.
    ///
    /// Five-step pipeline: get descriptor → create function → deactivate
    /// `inactive` parameters + set inputs → invoke → read outputs. The
    /// function handle is destroyed on Drop regardless of success or
    /// failure.
    ///
    /// Each name in `inactive` is deactivated for this call via
    /// `RfcSetParameterActive(…, 0)` (guide §4.2.2): an inactive input is
    /// not sent (so a backend-side default value applies), and an inactive
    /// output is not returned by the backend and is omitted from the
    /// result map. Unknown names fail with the SDK's error, like any
    /// unknown parameter name.
    ///
    /// The SDK reads every name as a NUL-terminated string. A function,
    /// parameter, field or `inactive` name with an embedded NUL fails with
    /// [`RfcError::InvalidInput`] before the SDK call that takes it, so
    /// before the unknown-name error. A text VALUE goes to the SDK with an
    /// explicit length and is not checked.
    pub fn call(
        &self,
        function_name: &str,
        inputs: &IndexMap<String, RfcValue>,
        inactive: &[String],
        compat: CompatOptions,
    ) -> Result<IndexMap<String, RfcValue>, RfcError> {
        // The wrapper abandons the handle when the SDK closes the
        // connection as part of an error (guide Table 4-A) — from any step
        // of the pipeline (a communication failure can surface in the
        // descriptor fetch just as well as in the invoke).
        self.abandon_if_fatal(self.call_impl(function_name, inputs, inactive, compat))
    }

    fn call_impl(
        &self,
        function_name: &str,
        inputs: &IndexMap<String, RfcValue>,
        inactive: &[String],
        compat: CompatOptions,
    ) -> Result<IndexMap<String, RfcValue>, RfcError> {
        let (func_desc, func_handle) =
            self.invoke_prepared(function_name, inputs, inactive, compat)?;

        // 5. Read outputs from the function handle (skips IMPORT and inactive
        // parameters unless compat asks for pyrfc's behaviour).
        marshal::read_outputs(ffi::sdk()?, func_desc, func_handle.raw(), compat)
    }

    /// Invoke an RFC function module like [`Connection::call`], but leave
    /// every top-level output table in the SDK container, to be taken in
    /// chunks through a [`TableReader`](crate::TableReader).
    ///
    /// Same parameters, same inputs pipeline, same outputs: the map has the
    /// keys `call` would return, in the same order. A top-level TABLE output
    /// is a [`ChunkedOutput::Table`]; every other output is a
    /// [`ChunkedOutput::Value`] holding exactly what `call` returns for it.
    /// Tables nested inside a structure or a row are read in full.
    ///
    /// Use it when a result is too large to hold twice: `call` builds the
    /// whole value tree while the whole SDK container is still alive. Here
    /// the caller converts one chunk, drops it, and takes the next; each
    /// chunk's rows are deleted from the container as it is taken.
    ///
    /// The readers own the function container between them (it is freed
    /// when the last reader is dropped, or at once when the call returns no
    /// table) and do not borrow the connection: the connection is free for
    /// the next call while the tables are still being read.
    pub fn call_chunked(
        &self,
        function_name: &str,
        inputs: &IndexMap<String, RfcValue>,
        inactive: &[String],
        compat: CompatOptions,
    ) -> Result<IndexMap<String, ChunkedOutput>, RfcError> {
        // Same abandon rule as `call`, over the same pipeline steps.
        self.abandon_if_fatal(self.call_chunked_impl(function_name, inputs, inactive, compat))
    }

    fn call_chunked_impl(
        &self,
        function_name: &str,
        inputs: &IndexMap<String, RfcValue>,
        inactive: &[String],
        compat: CompatOptions,
    ) -> Result<IndexMap<String, ChunkedOutput>, RfcError> {
        let (func_desc, func_handle) =
            self.invoke_prepared(function_name, inputs, inactive, compat)?;
        chunked::open_outputs(func_desc, func_handle, compat)
    }

    /// Steps 1-4 of the call pipeline: get descriptor → create function →
    /// set inputs + deactivate → invoke. Returns the invoked container.
    fn invoke_prepared(
        &self,
        function_name: &str,
        inputs: &IndexMap<String, RfcValue>,
        inactive: &[String],
        compat: CompatOptions,
    ) -> Result<(ffi::RFC_FUNCTION_DESC_HANDLE, FuncHandle), RfcError> {
        let handle = self.cell.get();
        if handle.is_null() {
            return Err(RfcError::Other(RfcErrorInfo {
                message: "connection is closed".into(),
                ..Default::default()
            }));
        }

        let sdk = ffi::sdk()?;

        // 1. Get the function descriptor from the SDK's repository cache.
        let func_desc = self.get_function_desc(sdk, handle, function_name)?;

        // 2. Create a function handle (owned — cleaned up by FuncHandle::Drop).
        let func_handle = FuncHandle::new(sdk, func_desc)?;

        // 3. Set inputs, then deactivate parameters — in that order: setting
        // a value on an inactive parameter re-activates it (probed live —
        // deactivate-then-set sent the value), while deactivating after a
        // set is exactly what RfcSetParameterActive exists for per the
        // guide's "no way back" scenario (§4.2.2). Deactivation last makes
        // it always win.
        // Compat reverses the order (pyrfc's, see `deactivate_before_inputs`).
        if compat.deactivate_before_inputs {
            for name in inactive {
                set_parameter_inactive(sdk, func_desc, func_handle.raw(), name, compat)?;
            }
            marshal::set_inputs(sdk, func_desc, func_handle.raw(), inputs, compat)?;
        } else {
            marshal::set_inputs(sdk, func_desc, func_handle.raw(), inputs, compat)?;
            for name in inactive {
                set_parameter_inactive(sdk, func_desc, func_handle.raw(), name, compat)?;
            }
        }

        // 4. Invoke the function on the SAP system.
        Self::invoke(sdk, handle, func_handle.raw())?;

        Ok((func_desc, func_handle))
    }

    fn get_function_desc(
        &self,
        sdk: &ffi::SapNwRfc,
        handle: ffi::RFC_CONNECTION_HANDLE,
        function_name: &str,
    ) -> Result<ffi::RFC_FUNCTION_DESC_HANDLE, RfcError> {
        let name_uc = crate::uc::encode_nul_terminated(function_name, || "function name".into())?;
        let mut error_info = ffi::zero_error_info();

        // Trap: concurrent cold lookups corrupt the SDK's shared metadata
        // cache (crash at process exit). See `ffi::lock_metadata_cache`.
        let _cache = ffi::lock_metadata_cache();

        // Safety: handle is a valid connection handle, name_uc is
        // null-terminated. The returned descriptor is cached by the SDK
        // (not owned by us — do not free).
        let desc = unsafe { sdk.RfcGetFunctionDesc(handle, name_uc.as_ptr(), &mut error_info) };

        if desc.is_null() {
            let info = ffi::extract_error_info("RfcGetFunctionDesc", &error_info);
            return Err(RfcError::from_error_info(info));
        }

        Ok(desc)
    }

    fn invoke(
        sdk: &ffi::SapNwRfc,
        handle: ffi::RFC_CONNECTION_HANDLE,
        func_handle: ffi::RFC_FUNCTION_HANDLE,
    ) -> Result<(), RfcError> {
        let mut error_info = ffi::zero_error_info();

        // Safety: handle is a valid connection handle, func_handle
        // is a valid function handle with inputs populated.
        let rc = unsafe { sdk.RfcInvoke(handle, func_handle, &mut error_info) };

        if rc != ffi::RFC_RC::RFC_OK {
            let info = ffi::extract_error_info("RfcInvoke", &error_info);
            return Err(RfcError::from_error_info(info));
        }

        Ok(())
    }
}

/// Deactivate one named parameter on a function data container for the
/// duration of this call (`RfcSetParameterActive`, guide §4.2.2).
///
/// An unknown name is rejected by our own `RfcGetParameterDescByName`
/// lookup first, because `RfcSetParameterActive` itself reports an unknown
/// name only from SDK 7.50 PL 15 on; below that it returns `RFC_OK` and the
/// name is silently ignored. The lookup makes native raise on every patch
/// level. Compat skips it (`CompatOptions::skip_inactive_name_check`).
fn set_parameter_inactive(
    sdk: &ffi::SapNwRfc,
    func_desc: ffi::RFC_FUNCTION_DESC_HANDLE,
    func_handle: ffi::RFC_FUNCTION_HANDLE,
    name: &str,
    compat: CompatOptions,
) -> Result<(), RfcError> {
    let name_uc = crate::uc::encode_nul_terminated(name, || "parameter name".into())?;
    if !compat.skip_inactive_name_check {
        marshal::parameter_desc(sdk, func_desc, &name_uc)?;
    }
    let mut error_info = ffi::zero_error_info();
    // Safety: func_handle is a valid function handle, name_uc is
    // null-terminated, error_info is a valid out-param.
    let rc =
        unsafe { sdk.RfcSetParameterActive(func_handle, name_uc.as_ptr(), 0, &mut error_info) };
    if rc != ffi::RFC_RC::RFC_OK {
        let info = ffi::extract_error_info("RfcSetParameterActive", &error_info);
        return Err(RfcError::from_error_info(info));
    }
    Ok(())
}

/// Owned function handle — `Drop` calls `RfcDestroyFunction`.
pub(crate) struct FuncHandle {
    handle: ffi::RFC_FUNCTION_HANDLE,
}

// Safety: RFC_FUNCTION_HANDLE points to an SDK data container that is not
// tied to the thread that created it. The SDK forbids using one container
// from two threads at the same time (guide, thread-safety note); moving it
// between threads is allowed. `FuncHandle` is not `Sync`, so shared access
// must go through a lock (`chunked` keeps it in a `Mutex`).
unsafe impl Send for FuncHandle {}

impl FuncHandle {
    /// Take ownership of a container created elsewhere (the offline
    /// repository tier). It is destroyed when the `FuncHandle` drops.
    pub(crate) fn from_raw(handle: ffi::RFC_FUNCTION_HANDLE) -> Self {
        FuncHandle { handle }
    }

    fn new(
        sdk: &ffi::SapNwRfc,
        func_desc: ffi::RFC_FUNCTION_DESC_HANDLE,
    ) -> Result<Self, RfcError> {
        let mut error_info = ffi::zero_error_info();

        // Safety: func_desc is a valid function descriptor handle.
        let handle = unsafe { sdk.RfcCreateFunction(func_desc, &mut error_info) };

        if handle.is_null() {
            let info = ffi::extract_error_info("RfcCreateFunction", &error_info);
            return Err(RfcError::from_error_info(info));
        }

        Ok(FuncHandle { handle })
    }

    pub(crate) fn raw(&self) -> ffi::RFC_FUNCTION_HANDLE {
        self.handle
    }
}

impl Drop for FuncHandle {
    fn drop(&mut self) {
        if self.handle.is_null() {
            return;
        }
        if let Ok(sdk) = ffi::sdk() {
            let mut error_info = ffi::zero_error_info();
            // Safety: handle is non-null and was returned by RfcCreateFunction.
            unsafe {
                sdk.RfcDestroyFunction(self.handle, &mut error_info);
            }
        }
        self.handle = std::ptr::null_mut();
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Best-effort close; errors are ignored on drop (constraint #6).
        // The lock is held across RfcCloseConnection so a concurrent
        // CancelToken::cancel can never reach a freed handle; the cell may
        // outlive the Connection through tokens, hence the null-out.
        if let Ok(sdk) = ffi::sdk() {
            let mut guard = self
                .cell
                .0
                .lock()
                .expect("connection handle mutex poisoned");
            if guard.is_null() {
                return;
            }
            let mut error_info = ffi::zero_error_info();
            // Safety: *guard is non-null and was returned by RfcOpenConnection.
            unsafe {
                sdk.RfcCloseConnection(*guard, &mut error_info);
            }
            *guard = std::ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_params_builder() {
        let p = ConnectionParams::new()
            .set("ashost", "sap01")
            .set("sysnr", "00")
            .set("client", "100");
        assert_eq!(p.len(), 3);
        assert!(!p.is_empty());

        let pairs: Vec<_> = p.iter().collect();
        assert_eq!(pairs[0], ("ashost", "sap01"));
        assert_eq!(pairs[1], ("sysnr", "00"));
        assert_eq!(pairs[2], ("client", "100"));
    }

    #[test]
    fn connection_params_empty() {
        let p = ConnectionParams::new();
        assert_eq!(p.len(), 0);
        assert!(p.is_empty());
        assert_eq!(p.iter().count(), 0);
    }

    #[test]
    fn connection_params_clone() {
        let p1 = ConnectionParams::new().set("user", "admin");
        let p2 = p1.clone();
        assert_eq!(p2.len(), 1);
        assert_eq!(p2.iter().next(), Some(("user", "admin")));
    }

    #[test]
    fn compat_options_default_all_false() {
        let opts = CompatOptions::default();
        assert!(!opts.skip_empty_date_time_inputs);
        assert!(!opts.skip_input_length_check);
        assert!(!opts.pyrfc_int_setters);
        assert!(!opts.skip_decimal_input_check);
        assert!(!opts.skip_inactive_name_check);
        assert!(!opts.deactivate_before_inputs);
        assert!(!opts.read_inactive_outputs);
        assert!(!opts.pyrfc_date_time_setters);
        assert!(!opts.return_import_params);
    }

    #[test]
    fn compat_options_copy() {
        let opts = CompatOptions {
            skip_empty_date_time_inputs: true,
            skip_input_length_check: true,
            pyrfc_int_setters: true,
            skip_decimal_input_check: true,
            skip_inactive_name_check: true,
            deactivate_before_inputs: true,
            read_inactive_outputs: true,
            pyrfc_date_time_setters: true,
            return_import_params: true,
        };
        let copy = opts;
        assert!(copy.skip_empty_date_time_inputs);
        assert!(copy.skip_input_length_check);
        assert!(copy.pyrfc_int_setters);
        assert!(copy.skip_decimal_input_check);
        assert!(copy.skip_inactive_name_check);
        assert!(copy.return_import_params);
    }
}
