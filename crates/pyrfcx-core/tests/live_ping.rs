// Live integration tests for Connection::ping() against SAP destination
// TEST_NW750, plus a probe dump of the SDK's RfcGetVersion description string.
//
// All tests are #[ignore] — run with: cargo test -- --ignored

use std::sync::Once;

use pyrfcx_core::{Connection, ConnectionParams};

static INI_INIT: Once = Once::new();

/// Point the SDK at the workspace root's sapnwrfc.ini.
/// Cargo integration tests run with cwd = package manifest dir, not workspace root.
fn ensure_ini_path() {
    INI_INIT.call_once(|| {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        pyrfcx_core::ffi::set_ini_path(workspace_root.to_str().unwrap())
            .expect("RfcSetIniPath failed");
    });
}

fn connect() -> Connection {
    ensure_ini_path();
    Connection::open(&ConnectionParams::new().set("dest", "TEST_NW750"))
        .expect("failed to connect to TEST_NW750")
}

#[test]
#[ignore]
fn ping_succeeds_on_open_connection() {
    let conn = connect();
    conn.ping().expect("ping failed on a live connection");
    // Still usable afterwards — ping must not consume the connection.
    conn.ping().expect("second ping failed");
    let a = conn.attributes().expect("attributes after ping failed");
    assert_eq!(a.rfc_role, "C");
}

#[test]
#[ignore]
fn ping_on_closed_connection_errors() {
    let mut conn = connect();
    conn.close().expect("close failed");
    let err = conn.ping().expect_err("expected an error");
    assert!(
        err.to_string().contains("closed"),
        "unexpected error: {err}"
    );
}

/// Probe: what does RfcGetVersion's description string actually contain?
/// The header only says "version information in string format".
#[test]
#[ignore]
fn version_probe_dump() {
    ensure_ini_path();
    let (major, minor, patch, description) =
        pyrfcx_core::ffi::get_version().expect("get_version failed");
    println!("\n=== RfcGetVersion ===");
    println!("major: {major}, minor: {minor}, patch_level: {patch}");
    println!("description: [{description}]");
    // The header's example ("*majorVersion = 7500") pins the convention:
    // major is the release, not a single digit.
    assert!(major >= 1000, "major is the SAP release, e.g. 7500");
}
