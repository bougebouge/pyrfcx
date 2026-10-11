// Live integration tests for Connection::attributes() against SAP destination
// TEST_NW750.
//
// All tests are #[ignore] — run with: cargo test -- --ignored
//
// Doubles as the probe for how the SDK fills RFC_ATTRIBUTES: the header
// declares every member SAP_UC name[N+1], which implies null-termination, but
// whether the SDK also blank-pads to N is a live fact, not a documented one.
// `dump_attributes` delimits each value so padding is visible.

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
fn attributes_probe_dump() {
    let conn = connect();
    let a = conn.attributes().expect("attributes failed");
    // Delimited so trailing blanks are visible in the output.
    let fields: [(&str, &str); 24] = [
        ("dest", &a.dest),
        ("host", &a.host),
        ("partner_host", &a.partner_host),
        ("sys_number", &a.sys_number),
        ("sys_id", &a.sys_id),
        ("client", &a.client),
        ("user", &a.user),
        ("language", &a.language),
        ("trace", &a.trace),
        ("iso_language", &a.iso_language),
        ("codepage", &a.codepage),
        ("partner_codepage", &a.partner_codepage),
        ("rfc_role", &a.rfc_role),
        ("type", &a.r#type),
        ("partner_type", &a.partner_type),
        ("rel", &a.rel),
        ("partner_rel", &a.partner_rel),
        ("kernel_rel", &a.kernel_rel),
        ("cpic_conv_id", &a.cpic_conv_id),
        ("prog_name", &a.prog_name),
        ("partner_bytes_per_char", &a.partner_bytes_per_char),
        ("partner_system_codepage", &a.partner_system_codepage),
        ("partner_ip", &a.partner_ip),
        ("partner_ipv6", &a.partner_ipv6),
    ];
    println!("\n=== RFC_ATTRIBUTES (TEST_NW750) ===");
    for (name, value) in fields {
        println!("{name:>24}: [{value}] (len {})", value.chars().count());
    }
}

#[test]
#[ignore]
fn attributes_describe_the_connection() {
    let conn = connect();
    let a = conn.attributes().expect("attributes failed");

    // Portable facts: true of any client connection to any R/3 backend.
    assert_eq!(a.rfc_role, "C", "we are the RFC client");
    assert_eq!(a.r#type, "E", "we are an external program");
    assert_eq!(a.partner_type, "3", "partner is an R/3 system");

    // `dest` is EMPTY even though we opened with dest=TEST_NW750 — the SDK
    // does not echo the destination back (probe, 750 PL 18). Pinned because it
    // is surprising, not because it is desirable.
    assert_eq!(a.dest, "", "SDK does not report the destination we opened");

    // System-local: the SID is checked only when PYRFCX_TEST_SID names it.
    match std::env::var("PYRFCX_TEST_SID") {
        Ok(sid) => assert_eq!(a.sys_id, sid),
        Err(_) => assert_eq!(a.sys_id.chars().count(), 3, "a SID has 3 characters"),
    }
    assert_eq!(a.client, "040");
    assert_eq!(a.sys_number, "10");
    assert!(!a.partner_host.is_empty(), "partner host is reported");
    assert_eq!(a.partner_rel, "750", "trailing pad blank stripped");
}

#[test]
#[ignore]
fn attributes_on_closed_connection_errors() {
    let mut conn = connect();
    conn.close().expect("close failed");
    let err = conn.attributes().expect_err("expected an error");
    assert!(
        err.to_string().contains("closed"),
        "unexpected error: {err}"
    );
}
