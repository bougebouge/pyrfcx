// Integration test for the SDK loader failure path.
//
// Tests that load_sdk() produces correct SdkNotFound errors when the
// SDK library is absent. Uses try_load_sdk() to bypass the OnceLock
// singleton — each call creates a fresh load attempt.
//
// All cases run in a single #[test] to avoid env var races (tests in
// the same binary run in parallel; env vars are process-global).

use pyrfcx_core::RfcError;

/// Helper: temporarily sets SAPNWRFC_HOME, runs the closure, then restores.
///
/// # Safety
///
/// `set_var`/`remove_var` are unsafe in edition 2024 (not thread-safe).
/// Safe here because all callers run in a single #[test], sequentially.
fn with_env(key: &str, value: Option<&str>, f: impl FnOnce()) {
    let prev = std::env::var(key).ok();
    // Safety: single-threaded within this test function; no concurrent env access.
    unsafe {
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
    f();
    unsafe {
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}

#[test]
fn loader_failure_paths() {
    // Case 1: nonexistent directory
    with_env(
        "SAPNWRFC_HOME",
        Some("C:\\nonexistent_pyrfcx_test_path_42"),
        || {
            let err = pyrfcx_core::ffi::try_load_sdk().unwrap_err();
            let msg = err.to_string();
            assert!(matches!(err, RfcError::SdkNotFound(_)));
            assert!(
                msg.contains("nonexistent_pyrfcx_test_path_42"),
                "should show the path tried: {msg}",
            );
            assert!(msg.contains("sapnwrfc"), "should name the library: {msg}",);
            assert!(
                msg.contains("SAPNWRFC_HOME"),
                "should mention env var: {msg}",
            );
            assert!(
                msg.contains("Set SAPNWRFC_HOME"),
                "should include the help pointer: {msg}",
            );
        },
    );

    // Case 2: SAPNWRFC_HOME unset — may succeed if SDK is on PATH
    with_env("SAPNWRFC_HOME", None, || {
        match pyrfcx_core::ffi::try_load_sdk() {
            Ok(()) => {
                // SDK found via platform default search — valid on machines
                // where sapnwrfc.dll is in PATH.
            }
            Err(err) => {
                let msg = err.to_string();
                assert!(matches!(err, RfcError::SdkNotFound(_)));
                assert!(
                    msg.contains("not set"),
                    "should say SAPNWRFC_HOME is not set: {msg}",
                );
            }
        }
    });
}
