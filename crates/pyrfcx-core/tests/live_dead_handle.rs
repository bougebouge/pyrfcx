// Regression test + probe record: handles the SDK has already closed
// through a fatal error must be ABANDONED, never passed to another SDK
// call.
//
// The original probe (2026-07-17, vendored DLL 750 PL 18, TEST_NW750)
// CONFIRMED the hazard this guards against: after a connection-fatal error
// (here induced by RfcCancel) the SDK frees the handle and reuses the
// address for the very next RfcOpenConnection — the first fresh connection
// landed on the dead handle's address — and RfcCloseConnection on the
// stale pointer closed that innocent live connection (its ping then failed
// with RFC_INVALID_HANDLE). That use-after-free was the root cause of the
// intermittent `TestEdgeCases` failure: any
// dead-but-not-abandoned connection finalized later (e.g. by Python GC) could kill whatever connection currently owned the
// reused address.
//
// The fix: `RfcError::sdk_closed_connection` (guide Table 4-A) +
// `HandleCell::abandon` — the core nulls the handle without closing it.
// This test re-runs the original probe scenario and asserts the fixed
// behavior end to end.
//
// Run with: cargo test -p pyrfcx-core --test live_dead_handle -- --ignored --nocapture

use std::sync::Once;
use std::time::Duration;

use indexmap::IndexMap;

use pyrfcx_core::{CompatOptions, Connection, ConnectionParams, RfcValue};

static INI_INIT: Once = Once::new();

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

/// Kill a connection SDK-side via cancel (comm-failure family, no ST22
/// noise) and return it un-dropped, together with its pre-death address.
fn make_dead_connection() -> (Connection, usize) {
    let conn = connect();
    let addr = conn.handle() as usize;
    let token = conn.cancel_token();
    let worker = std::thread::spawn(move || {
        let mut inputs = IndexMap::new();
        inputs.insert("SECONDS".into(), RfcValue::Int(10));
        let _ = conn.call("RFC_PING_AND_WAIT", &inputs, &[], CompatOptions::default());
        conn
    });
    std::thread::sleep(Duration::from_millis(1500));
    token.cancel().expect("cancel failed");
    (worker.join().expect("worker panicked"), addr)
}

#[test]
#[ignore]
fn fatal_error_abandons_the_handle_and_spares_reused_addresses() {
    // A: killed SDK-side; the fix must have abandoned its handle already.
    let (dead, dead_addr) = make_dead_connection();
    println!("dead handle was: {dead_addr:#x}");
    assert!(
        dead.handle().is_null(),
        "fatal error must abandon the handle (sdk_closed_connection)"
    );

    // B: open new connections until one lands on the dead one's address
    // (the original probe saw reuse on the very first open), keeping the
    // misses alive so the allocator can't hand the same block back twice.
    let mut kept = Vec::new();
    let mut victim = None;
    for i in 0..32 {
        let c = connect();
        let addr = c.handle() as usize;
        if addr == dead_addr {
            println!("fresh #{i} reused the address");
            victim = Some(c);
            break;
        }
        kept.push(c);
    }

    let Some(victim) = victim else {
        println!("no address reuse within 32 opens — reuse assertion skipped this run");
        drop(dead);
        return;
    };

    // Dropping the dead connection must now be a no-op — before the fix
    // this closed the victim's live handle (probe: ping then failed with
    // RFC_INVALID_HANDLE).
    drop(dead);

    victim
        .ping()
        .expect("victim must survive the dead connection's drop");
    println!("victim survived — abandoned handle was never passed back to the SDK");
}
