// Live thread-safety stress tests against SAP destination TEST_NW750.
//
// The SDK keeps process-wide state behind every handle: the metadata
// cache and the ini/crypto configuration. pyrfcx guards both with its own
// locks (`ffi::lock_metadata_cache`, the SDK config lock). These tests run
// many threads against that state at once. A broken cache does not fail a
// test: the process dies at exit, after every test passed. So the pass
// criterion of this binary is its exit code, checked over many runs:
//
//   for i in $(seq 100); do target/debug/deps/live_threads-<hash> --ignored \
//     || echo "run $i: exit $?"; done
//
// All tests are #[ignore] — run with: cargo test --test live_threads -- --ignored

#![allow(clippy::result_large_err)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Once};
use std::thread;

use indexmap::IndexMap;
use pyrfcx_core::{ChunkedOutput, CompatOptions, Connection, ConnectionParams, RfcValue};

static INI_INIT: Once = Once::new();

fn workspace_root() -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_str()
        .expect("utf-8 path")
        .to_owned()
}

fn init_ini() {
    INI_INIT.call_once(|| {
        pyrfcx_core::ffi::set_ini_path(&workspace_root()).expect("RfcSetIniPath failed");
    });
}

fn connect() -> Connection {
    Connection::open(&ConnectionParams::new().set("dest", "TEST_NW750"))
        .expect("failed to connect to TEST_NW750")
}

fn s(v: &str) -> RfcValue {
    RfcValue::String(v.into())
}

/// One function module and the inputs to call it with. `exists: false`
/// is a name the backend does not know: lookup and call must fail.
struct Case {
    name: &'static str,
    inputs: IndexMap<String, RfcValue>,
    exists: bool,
}

fn case(name: &'static str, inputs: &[(&str, RfcValue)]) -> Case {
    Case {
        name,
        inputs: inputs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
        exists: true,
    }
}

fn mem_rows_inputs(rows: i64) -> IndexMap<String, RfcValue> {
    IndexMap::from([
        ("IV_ROWS".into(), RfcValue::Int(rows.into())),
        ("IV_FILL".into(), RfcValue::Int(15)),
    ])
}

/// Function modules no other test in this binary looks up, so the first
/// lookup of each is a cold cache miss.
fn cold_cases() -> Vec<Case> {
    let deep_row = IndexMap::from([
        ("I".into(), RfcValue::Int(1)),
        ("C".into(), s("Row1")),
        ("STR".into(), s("string 1")),
        ("XSTR".into(), RfcValue::Bytes(vec![0xAB])),
    ]);
    vec![
        case("STFC_CONNECTION", &[("REQUTEXT", s("threads"))]),
        case("STFC_STRUCTURE", &[]),
        case(
            "STFC_DEEP_TABLE",
            &[("IMPORT_TAB", RfcValue::Table(vec![deep_row]))],
        ),
        case("RFC_PING", &[]),
        case("RFC_SYSTEM_INFO", &[]),
        case("BAPI_USER_GET_DETAIL", &[("USERNAME", s("ZZNOUSER"))]),
        case(
            "ZPYRFCX_MEM_ROWS",
            &[
                ("IV_ROWS", RfcValue::Int(5)),
                ("IV_FILL", RfcValue::Int(15)),
            ],
        ),
        Case {
            name: "ZPYRFCX_NO_SUCH_FUNCTION",
            inputs: IndexMap::new(),
            exists: false,
        },
    ]
}

/// Lookup, call and describe `c` on `conn`; for an unknown name, all three
/// must fail with the SDK's error.
fn exercise(conn: &Connection, c: &Case) {
    let call = conn.call(c.name, &c.inputs, &[], CompatOptions::default());
    let desc = conn.describe_function(c.name);
    if c.exists {
        call.unwrap_or_else(|e| panic!("{}: call failed: {e}", c.name));
        let desc = desc.unwrap_or_else(|e| panic!("{}: describe failed: {e}", c.name));
        assert_eq!(desc.name, c.name);
    } else {
        assert!(call.is_err(), "{}: call must fail", c.name);
        assert!(desc.is_err(), "{}: describe must fail", c.name);
    }
}

/// `call_chunked` on ZPYRFCX_MEM_ROWS, the table taken in small chunks
/// from the last row. Returns the number of rows taken.
fn read_chunked(conn: &Connection, rows: i64) -> usize {
    let mut outs = conn
        .call_chunked(
            "ZPYRFCX_MEM_ROWS",
            &mem_rows_inputs(rows),
            &[],
            CompatOptions::default(),
        )
        .expect("call_chunked");
    let Some(ChunkedOutput::Table(mut reader)) = outs.shift_remove("ET_ROWS") else {
        panic!("ET_ROWS is not a table reader");
    };
    let mut taken = 0;
    while reader.remaining() > 0 {
        let chunk = reader.take_last_rows(7).expect("take rows");
        taken += chunk.len();
    }
    taken
}

/// 16 threads open their own connections at once, wait until all are
/// open, then all look up the same functions in the same order, so every
/// name is a cold miss for 16 threads at the same moment (the pattern that
/// broke the SDK cache). Each thread calls and describes every
/// function, reads a small table in chunks and closes.
#[test]
#[ignore]
fn cold_lookups_on_many_connections() {
    const THREADS: usize = 16;
    init_ini();
    let cases = Arc::new(cold_cases());
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let cases = Arc::clone(&cases);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let mut conn = connect();
                // The opens take different times; start the lookups together.
                barrier.wait();
                for c in cases.iter() {
                    exercise(&conn, c);
                }
                assert_eq!(read_chunked(&conn, 40), 40);
                conn.ping().expect("ping after the round");
                conn.close().expect("close");
            })
        })
        .collect();
    for h in handles {
        h.join().expect("worker thread panicked");
    }
}

/// Function modules only this test looks up.
fn ini_cases() -> Vec<Case> {
    vec![
        case("STFC_STRING", &[("QUESTION", s("threads"))]),
        case("STFC_XSTRING", &[]),
        case(
            "RFC_READ_TABLE",
            &[("QUERY_TABLE", s("T000")), ("ROWCOUNT", RfcValue::Int(3))],
        ),
        case("RFC_PING_AND_WAIT", &[("SECONDS", RfcValue::Int(0))]),
        case(
            "STFC_CHANGING",
            &[
                ("START_VALUE", RfcValue::Int(1)),
                ("COUNTER", RfcValue::Int(0)),
            ],
        ),
    ]
}

/// Opens, cold lookups and calls on 8 threads while one more thread reads
/// `sapnwrfc.ini` again and again (`RfcReloadIniFile`, `RfcSetIniPath` to
/// the same directory). Every open must find the destination: the config
/// lock keeps a reload from running inside an open.
#[test]
#[ignore]
fn ini_reload_during_opens_and_lookups() {
    const WORKERS: usize = 8;
    const ROUNDS: usize = 3;
    init_ini();
    let cases = Arc::new(ini_cases());
    let barrier = Arc::new(Barrier::new(WORKERS + 1));
    let stop = Arc::new(AtomicBool::new(false));
    let reloads = Arc::new(AtomicUsize::new(0));

    let reloader = {
        let barrier = Arc::clone(&barrier);
        let stop = Arc::clone(&stop);
        let reloads = Arc::clone(&reloads);
        let root = workspace_root();
        thread::spawn(move || {
            barrier.wait();
            while !stop.load(Ordering::Relaxed) {
                pyrfcx_core::ffi::reload_ini_file().expect("RfcReloadIniFile");
                pyrfcx_core::ffi::set_ini_path(&root).expect("RfcSetIniPath");
                reloads.fetch_add(1, Ordering::Relaxed);
                thread::yield_now();
            }
        })
    };

    let workers: Vec<_> = (0..WORKERS)
        .map(|i| {
            let cases = Arc::clone(&cases);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                for round in 0..ROUNDS {
                    let mut conn = connect();
                    exercise(&conn, &cases[(i + round) % cases.len()]);
                    conn.close().expect("close");
                }
            })
        })
        .collect();

    let mut panicked = false;
    for w in workers {
        panicked |= w.join().is_err();
    }
    stop.store(true, Ordering::Relaxed);
    reloader.join().expect("reloader thread panicked");
    assert!(!panicked, "a worker thread panicked");
    assert!(
        reloads.load(Ordering::Relaxed) > 0,
        "the reloader never ran"
    );
}
