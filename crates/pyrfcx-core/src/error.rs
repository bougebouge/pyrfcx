// Error types for pyrfcx-core: decoded RFC_ERROR_INFO and classified error enum.

/// Decoded contents of `RFC_ERROR_INFO`.
///
/// Every field is decoded from UTF-16 to a Rust String at the point of
/// error creation. Fields that are empty in the SDK struct become empty
/// strings (not `Option` — there is no semantic "absent" vs "empty").
#[derive(Debug, Clone, Default)]
pub struct RfcErrorInfo {
    /// `RFC_RC` return code (e.g., 1 = communication failure, 5 = ABAP exception).
    pub code: u32,
    /// `RFC_ERROR_GROUP` classification (e.g., 1 = ABAP_APPLICATION_FAILURE).
    pub group: u32,
    /// Exception key (e.g., `"FU_NOT_FOUND"`, `"NOT_AUTHORIZED"`).
    pub key: String,
    /// Human-readable error message.
    pub message: String,
    /// ABAP message class (SY-MSGID).
    pub abap_msg_class: String,
    /// ABAP message type (`"E"`, `"A"`, `"X"`, etc.).
    pub abap_msg_type: String,
    /// ABAP message number (SY-MSGNO), as a string of digits.
    pub abap_msg_number: String,
    /// ABAP message variable 1 (SY-MSGV1).
    pub abap_msg_v1: String,
    /// ABAP message variable 2 (SY-MSGV2).
    pub abap_msg_v2: String,
    /// ABAP message variable 3 (SY-MSGV3).
    pub abap_msg_v3: String,
    /// ABAP message variable 4 (SY-MSGV4).
    pub abap_msg_v4: String,
    /// Name of the SDK function that returned this error, e.g.
    /// `"RfcInvoke"` or `"RfcGetFunctionDesc"`. Plain metadata: it tells a
    /// caller which step of an operation failed. Empty when the error was
    /// raised by our own code before any SDK call (a closed connection).
    pub sdk_function: &'static str,
}

/// Errors from the SAP NW RFC SDK, classified by error group — plus
/// `InvalidInput`/`InvalidType` for caller misuse detected by our own
/// code before any SDK call.
///
/// The four main variants correspond to the SDK's `RFC_ERROR_GROUP`
/// values that matter for client calls; `Other` catches everything else.
/// `SdkNotFound`, `InvalidInput` and `InvalidType` are the only
/// variants not backed by an `RfcErrorInfo`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RfcError {
    /// Network or transport failure.
    /// Error group `COMMUNICATION_FAILURE` (4).
    #[error("communication failure: {}", .0.message)]
    CommunicationFailure(RfcErrorInfo),

    /// Logon or SNC failure.
    /// Error group `LOGON_FAILURE` (3).
    #[error("logon failure: {}", .0.message)]
    LogonFailure(RfcErrorInfo),

    /// ABAP SYSTEM_FAILURE or ABAP MESSAGE (E/A/X).
    /// Error group `ABAP_RUNTIME_FAILURE` (2).
    #[error("ABAP runtime error: {}", .0.message)]
    AbapRuntimeFailure(RfcErrorInfo),

    /// ABAP exception raised by the function module (RAISE / MESSAGE…RAISING).
    /// Error group `ABAP_APPLICATION_FAILURE` (1).
    #[error("ABAP exception {}: {}", .0.key, .0.message)]
    AbapException(RfcErrorInfo),

    /// Any SDK error not classified into the four groups above.
    #[error("RFC error (code {}, group {}): {}", .0.code, .0.group, .0.message)]
    Other(RfcErrorInfo),

    /// Caller misuse detected by the marshaller before any SDK call —
    /// input exceeding a fixed-length field's width, an integer
    /// out of the target width's range, a value supplied for
    /// an EXPORT-only parameter, or a path with an embedded NUL
    /// for a process-wide SDK setting. Mapped to stdlib `ValueError`
    /// by pyrfcx-py, never to the `PyrfcxError` tree (the class
    /// follows the fault, not where it is detected).
    #[error("{0}")]
    InvalidInput(String),

    /// Wrong value kind for the target field's RFCTYPE, detected before
    /// any SDK call — e.g. a string targeting an INT4 field, an int
    /// targeting CHAR, non-bytes targeting BYTE/XSTRING. Mapped
    /// to stdlib `TypeError` by pyrfcx-py (wrong type is
    /// `TypeError`, right type / wrong value is `ValueError`).
    #[error("{0}")]
    InvalidType(String),

    /// The SDK shared library could not be loaded or is missing symbols.
    #[error("{0}")]
    SdkNotFound(String),
}

impl RfcError {
    /// Classify an `RfcErrorInfo` into the appropriate error variant
    /// based on the `group` field.
    pub(crate) fn from_error_info(info: RfcErrorInfo) -> Self {
        match info.group {
            4 => RfcError::CommunicationFailure(info),
            3 => RfcError::LogonFailure(info),
            2 => RfcError::AbapRuntimeFailure(info),
            1 => RfcError::AbapException(info),
            _ => RfcError::Other(info),
        }
    }

    /// Whether the SDK has closed the connection as part of this error.
    ///
    /// Per the SDK guide's Table 4-A, the connection is closed after
    /// `RFC_COMMUNICATION_FAILURE` (code 1), `RFC_ABAP_RUNTIME_FAILURE`
    /// (3) and `RFC_ABAP_MESSAGE` (4) — and stays open after
    /// `RFC_ABAP_EXCEPTION`. `RFC_CANCELED` (7) closes it per the
    /// `RfcCancel` header doc (probed: it arrives as code 7 in the
    /// communication-failure group). The group arms cover the same ground
    /// for codes the table doesn't enumerate (e.g. `RFC_CLOSED`): any
    /// error in the communication or ABAP-runtime group means the
    /// connection is gone.
    ///
    /// This matters for memory safety, not just state tracking: the SDK
    /// frees the handle when it closes the connection this way, and
    /// reuses the address for new connections immediately — so the handle
    /// must be abandoned, never passed to another SDK call (not even
    /// `RfcCloseConnection`). Probe: `tests/live_dead_handle.rs`.
    pub fn sdk_closed_connection(&self) -> bool {
        match self.info() {
            Some(info) => {
                // Codes: 1 = RFC_COMMUNICATION_FAILURE, 3 = RFC_ABAP_RUNTIME_FAILURE,
                // 4 = RFC_ABAP_MESSAGE, 7 = RFC_CANCELED.
                // Groups: 2 = ABAP_RUNTIME_FAILURE, 4 = COMMUNICATION_FAILURE.
                matches!(info.code, 1 | 3 | 4 | 7) || matches!(info.group, 2 | 4)
            }
            None => false,
        }
    }

    /// The decoded `RFC_ERROR_INFO`, if this error carries one.
    /// Every variant except `SdkNotFound`, `InvalidInput` and
    /// `InvalidType` has one.
    pub fn info(&self) -> Option<&RfcErrorInfo> {
        match self {
            RfcError::SdkNotFound(_) | RfcError::InvalidInput(_) | RfcError::InvalidType(_) => None,
            RfcError::CommunicationFailure(i)
            | RfcError::LogonFailure(i)
            | RfcError::AbapRuntimeFailure(i)
            | RfcError::AbapException(i)
            | RfcError::Other(i) => Some(i),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_info(code: u32, group: u32, key: &str, message: &str) -> RfcErrorInfo {
        RfcErrorInfo {
            code,
            group,
            key: key.to_string(),
            message: message.to_string(),
            ..Default::default()
        }
    }

    // ── classification ──────────────────────────────────────────────

    #[test]
    fn classify_communication_failure() {
        let err = RfcError::from_error_info(make_info(1, 4, "", "connection reset"));
        assert!(matches!(err, RfcError::CommunicationFailure(_)));
    }

    #[test]
    fn classify_logon_failure() {
        let err = RfcError::from_error_info(make_info(2, 3, "", "bad password"));
        assert!(matches!(err, RfcError::LogonFailure(_)));
    }

    #[test]
    fn classify_abap_runtime_failure() {
        let err = RfcError::from_error_info(make_info(3, 2, "", "short dump"));
        assert!(matches!(err, RfcError::AbapRuntimeFailure(_)));
    }

    #[test]
    fn classify_abap_exception() {
        let err = RfcError::from_error_info(make_info(5, 1, "FU_NOT_FOUND", "not found"));
        assert!(matches!(err, RfcError::AbapException(_)));
    }

    // ── sdk_closed_connection (guide Table 4-A + RfcCancel probe) ───

    #[test]
    fn sdk_closed_connection_covers_table_4a_and_cancel() {
        // Closed: RFC_COMMUNICATION_FAILURE, RFC_ABAP_RUNTIME_FAILURE,
        // RFC_ABAP_MESSAGE, RFC_CANCELED (probed as code 7, group 4).
        let closed = [
            make_info(1, 4, "", "connection reset"),
            make_info(3, 2, "CONVT_OVERFLOW", "short dump"),
            make_info(4, 2, "", "abap message type E"),
            make_info(7, 4, "RFC_CANCELED", "Connection was canceled."),
        ];
        for info in closed {
            let err = RfcError::from_error_info(info.clone());
            assert!(err.sdk_closed_connection(), "should be fatal: {info:?}");
        }

        // Open: ABAP exception (Table 4-A: "Connection remains open"),
        // RFC_ILLEGAL_STATE (cancel with no call in flight, probed),
        // RFC_INVALID_PARAMETER (unknown parameter name, probed).
        let open = [
            make_info(5, 1, "FU_NOT_FOUND", "not found"),
            make_info(19, 5, "RFC_ILLEGAL_STATE", "not in call"),
            make_info(20, 5, "RFC_INVALID_PARAMETER", "field not found"),
        ];
        for info in open {
            let err = RfcError::from_error_info(info.clone());
            assert!(
                !err.sdk_closed_connection(),
                "should be non-fatal: {info:?}"
            );
        }

        // Variants without RFC_ERROR_INFO never abandon.
        assert!(!RfcError::InvalidInput("x".into()).sdk_closed_connection());
        assert!(!RfcError::SdkNotFound("x".into()).sdk_closed_connection());
    }

    #[test]
    fn classify_other_group_0() {
        let err = RfcError::from_error_info(make_info(0, 0, "", "ok"));
        assert!(matches!(err, RfcError::Other(_)));
    }

    #[test]
    fn classify_other_group_5() {
        let err = RfcError::from_error_info(make_info(15, 5, "", "external runtime"));
        assert!(matches!(err, RfcError::Other(_)));
    }

    #[test]
    fn classify_other_group_6() {
        let err = RfcError::from_error_info(make_info(15, 6, "", "external app"));
        assert!(matches!(err, RfcError::Other(_)));
    }

    #[test]
    fn classify_other_group_7() {
        let err = RfcError::from_error_info(make_info(15, 7, "", "external auth"));
        assert!(matches!(err, RfcError::Other(_)));
    }

    #[test]
    fn classify_other_group_99() {
        let err = RfcError::from_error_info(make_info(0, 99, "", "unknown"));
        assert!(matches!(err, RfcError::Other(_)));
    }

    // ── field preservation ──────────────────────────────────────────

    #[test]
    fn code_and_group_preserved() {
        let err = RfcError::from_error_info(make_info(5, 1, "FU_NOT_FOUND", "msg"));
        let info = err.info().unwrap();
        assert_eq!(info.code, 5);
        assert_eq!(info.group, 1);
        assert_eq!(info.key, "FU_NOT_FOUND");
        assert_eq!(info.message, "msg");
    }

    #[test]
    fn other_preserves_code_and_group() {
        let err = RfcError::from_error_info(make_info(22, 5, "", "ext"));
        let info = err.info().unwrap();
        assert_eq!(info.code, 22);
        assert_eq!(info.group, 5);
    }

    // ── info() accessor ─────────────────────────────────────────────

    #[test]
    fn info_returns_none_for_sdk_not_found() {
        let err = RfcError::SdkNotFound("missing".into());
        assert!(err.info().is_none());
    }

    #[test]
    fn info_returns_none_for_invalid_input() {
        let err = RfcError::InvalidInput("too long".into());
        assert!(err.info().is_none());
    }

    #[test]
    fn display_invalid_input() {
        let err = RfcError::InvalidInput("input for RFCCHAR4 exceeds the field width".into());
        assert_eq!(
            err.to_string(),
            "input for RFCCHAR4 exceeds the field width"
        );
    }

    // ── Display ─────────────────────────────────────────────────────

    #[test]
    fn display_communication_failure() {
        let err = RfcError::from_error_info(make_info(1, 4, "", "connection reset"));
        assert_eq!(err.to_string(), "communication failure: connection reset");
    }

    #[test]
    fn display_logon_failure() {
        let err = RfcError::from_error_info(make_info(2, 3, "", "bad password"));
        assert_eq!(err.to_string(), "logon failure: bad password");
    }

    #[test]
    fn display_abap_runtime() {
        let err = RfcError::from_error_info(make_info(3, 2, "", "short dump"));
        assert_eq!(err.to_string(), "ABAP runtime error: short dump");
    }

    #[test]
    fn display_abap_exception() {
        let err = RfcError::from_error_info(make_info(5, 1, "FU_NOT_FOUND", "not found"));
        assert_eq!(err.to_string(), "ABAP exception FU_NOT_FOUND: not found");
    }

    #[test]
    fn display_other() {
        let err = RfcError::from_error_info(make_info(22, 5, "", "external"));
        assert_eq!(err.to_string(), "RFC error (code 22, group 5): external");
    }

    #[test]
    fn display_sdk_not_found() {
        let err = RfcError::SdkNotFound("sapnwrfc.dll not found".into());
        assert_eq!(err.to_string(), "sapnwrfc.dll not found");
    }

    // ── sdk_function ────────────────────────────────────────────────

    #[test]
    fn sdk_function_is_carried_and_defaults_empty() {
        let raw = crate::ffi::zero_error_info();
        let err = RfcError::from_error_info(crate::ffi::extract_error_info("RfcInvoke", &raw));
        assert_eq!(err.info().unwrap().sdk_function, "RfcInvoke");
        // Errors raised by our own code before any SDK call carry no name.
        assert_eq!(RfcErrorInfo::default().sdk_function, "");
    }

    // ── Default ─────────────────────────────────────────────────────

    #[test]
    fn error_info_default_is_all_empty() {
        let info = RfcErrorInfo::default();
        assert_eq!(info.code, 0);
        assert_eq!(info.group, 0);
        assert_eq!(info.key, "");
        assert_eq!(info.message, "");
        assert_eq!(info.abap_msg_class, "");
        assert_eq!(info.abap_msg_v4, "");
    }
}
