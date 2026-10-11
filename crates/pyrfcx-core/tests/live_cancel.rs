// Live tests + probes for cross-thread call cancellation (RfcCancel)
// against SAP destination TEST_NW750.
//
// All tests are #[ignore] — run with: cargo test -- --ignored

use std::sync::Once;
use std::time::{Duration, Instant};

use indexmap::IndexMap;

use pyrfcx_core::{CompatOptions, Connection, ConnectionParams, RfcValue};

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

/// Probe: what does RFC_PING_AND_WAIT's signature look like on TEST_NW750?
/// (pyrfc's timeout suite uses it; we derive the parameter from the system
/// itself, not from pyrfc.)
#[test]
#[ignore]
fn probe_ping_and_wait_signature() {
    let conn = connect();
    let desc = conn
        .describe_function("RFC_PING_AND_WAIT")
        .expect("describe RFC_PING_AND_WAIT failed");
    println!("\n=== RFC_PING_AND_WAIT signature ===");
    for p in &desc.params {
        println!(
            "  {} dir={:?} type={:?} uc_length={}",
            p.name, p.direction, p.desc.rfctype, p.desc.uc_length
        );
    }
}

/// The main probe: cancel a call that would otherwise block for 10 seconds.
/// Records the error the cancelled RfcInvoke returns (code/group/key), that
/// the connection is closed afterwards, and that dropping it is safe.
#[test]
#[ignore]
fn cancel_interrupts_blocked_call() {
    let conn = connect();
    let token = conn.cancel_token();

    let started = Instant::now();
    let worker = std::thread::spawn(move || {
        let mut inputs = IndexMap::new();
        inputs.insert("SECONDS".into(), RfcValue::Int(10));
        let result = conn.call("RFC_PING_AND_WAIT", &inputs, &[], CompatOptions::default());
        (conn, result)
    });

    // Give the worker time to get past the descriptor fetch and into
    // RfcInvoke. If this ever races, cancel returns RFC_ILLEGAL_STATE and
    // the probe fails visibly — lengthen the sleep, don't loosen the assert.
    std::thread::sleep(Duration::from_millis(1500));
    token.cancel().expect("cancel of an in-flight call failed");

    let (conn, result) = worker.join().expect("worker panicked");
    let elapsed = started.elapsed();
    println!("\n=== cancelled call probe ===");
    println!("elapsed: {elapsed:?} (would have been ≥10s uncancelled)");
    let err = result.expect_err("cancelled call must error");
    println!("error variant: {err:?}");
    if let Some(info) = err.info() {
        println!(
            "code={} group={} key=[{}] message=[{}]",
            info.code, info.group, info.key, info.message
        );
    }
    assert!(
        elapsed < Duration::from_secs(9),
        "call was not actually interrupted (elapsed {elapsed:?})"
    );

    // Per the SDK header, cancelling closes the connection.
    let after = conn.attributes();
    println!("attributes() after cancel: {after:?}");
    assert!(after.is_err(), "connection should be closed after cancel");

    // Dropping the SDK-closed connection must be safe (same as after a
    // communication failure — the handle still needs RfcCloseConnection).
    drop(conn);
}

/// Probe: cancel with no call in flight — the header promises
/// RFC_ILLEGAL_STATE. Records whether the connection survives it.
#[test]
#[ignore]
fn cancel_without_call_in_flight() {
    let conn = connect();
    let token = conn.cancel_token();

    let err = token.cancel().expect_err("cancel with no call must error");
    println!("\n=== cancel-without-call probe ===");
    println!("error variant: {err:?}");
    if let Some(info) = err.info() {
        println!(
            "code={} group={} key=[{}] message=[{}]",
            info.code, info.group, info.key, info.message
        );
    }

    // Does the connection survive a failed cancel? Record by pinging.
    let ping = conn.ping();
    println!("ping after failed cancel: {ping:?}");
}

#[test]
#[ignore]
fn cancel_after_close_errors_cleanly() {
    let mut conn = connect();
    let token = conn.cancel_token();
    conn.close().expect("close failed");

    let err = token.cancel().expect_err("cancel on closed must error");
    assert!(
        err.to_string().contains("closed"),
        "unexpected error: {err}"
    );

    // Token also outlives Drop safely.
    let conn2 = connect();
    let token2 = conn2.cancel_token();
    drop(conn2);
    let err = token2.cancel().expect_err("cancel after drop must error");
    assert!(
        err.to_string().contains("closed"),
        "unexpected error: {err}"
    );
}
