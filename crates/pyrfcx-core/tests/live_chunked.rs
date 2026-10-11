// Live tests for Connection::call_chunked against SAP destination TEST_NW750:
// the outputs assembled from chunks equal the outputs of Connection::call.
//
// All tests are #[ignore] — run with: cargo test -- --ignored

#![allow(clippy::result_large_err)]

use std::sync::Once;

use indexmap::IndexMap;
use pyrfcx_core::{ChunkedOutput, CompatOptions, Connection, ConnectionParams, RfcValue};

static INI_INIT: Once = Once::new();

fn connect() -> Connection {
    INI_INIT.call_once(|| {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        pyrfcx_core::ffi::set_ini_path(workspace_root.to_str().unwrap())
            .expect("RfcSetIniPath failed");
    });
    Connection::open(&ConnectionParams::new().set("dest", "TEST_NW750"))
        .expect("failed to connect to TEST_NW750")
}

/// The options the compat facade passes (pyrfc-default config).
fn facade_compat() -> CompatOptions {
    CompatOptions {
        skip_empty_date_time_inputs: true,
        skip_input_length_check: true,
        pyrfc_int_setters: true,
        skip_decimal_input_check: true,
        deactivate_before_inputs: true,
        read_inactive_outputs: true,
        pyrfc_date_time_setters: true,
        return_import_params: false,
    }
}

/// Every chunk of every table taken with `chunk` rows at most, put back in
/// table order. The result has the shape `Connection::call` returns.
fn assemble(outputs: IndexMap<String, ChunkedOutput>, chunk: usize) -> IndexMap<String, RfcValue> {
    outputs
        .into_iter()
        .map(|(name, out)| {
            let value = match out {
                ChunkedOutput::Value(v) => v,
                ChunkedOutput::Table(mut reader) => {
                    let total = reader.row_count();
                    let mut chunks = Vec::new();
                    while reader.remaining() > 0 {
                        let c = reader.take_last_rows(chunk).expect("take_last_rows");
                        assert!(!c.is_empty() && c.len() <= chunk, "{name}: chunk size");
                        chunks.push(c);
                    }
                    let rows: Vec<_> = chunks.into_iter().rev().flatten().collect();
                    assert_eq!(rows.len(), total, "{name}: row count");
                    RfcValue::Table(rows)
                }
            };
            (name, value)
        })
        .collect()
}

/// Asserts call and call_chunked agree for every chunk size and both
/// option sets; returns the number of rows of `table` for the caller.
/// `varying` names outputs whose content changes from call to call (a
/// timestamp): only their variant and width are compared.
fn check_equal(
    conn: &Connection,
    fm: &str,
    inputs: &IndexMap<String, RfcValue>,
    table: &str,
    chunks: &[usize],
    varying: &[&str],
) -> usize {
    let mut rows = 0;
    for compat in [CompatOptions::default(), facade_compat()] {
        let expected = conn.call(fm, inputs, &[], compat).expect("call");
        rows = match expected.get(table) {
            Some(RfcValue::Table(r)) => r.len(),
            other => panic!("{table} is not a table: {other:?}"),
        };
        for &chunk in chunks {
            let outs = conn
                .call_chunked(fm, inputs, &[], compat)
                .expect("call_chunked");
            // Keys and order first, then the values.
            let keys: Vec<_> = outs.keys().cloned().collect();
            let expected_keys: Vec<_> = expected.keys().cloned().collect();
            assert_eq!(keys, expected_keys, "{fm}: output keys");
            assert!(matches!(outs.get(table), Some(ChunkedOutput::Table(_))));
            let mut got = assemble(outs, chunk);
            let mut expected = expected.clone();
            for name in varying {
                match (got.get_mut(*name), expected.get_mut(*name)) {
                    (Some(RfcValue::Char(a)), Some(RfcValue::Char(b))) => {
                        assert_eq!(a.chars().count(), b.chars().count(), "{name}: width");
                        a.clear();
                        b.clear();
                    }
                    other => panic!("{name}: expected two Char values, got {other:?}"),
                }
            }
            assert_eq!(got, expected, "{fm}: chunk size {chunk}");
        }
    }
    rows
}

fn deep_table_inputs() -> IndexMap<String, RfcValue> {
    let rows = (1..=2)
        .map(|i| {
            IndexMap::from([
                ("I".into(), RfcValue::Int(i)),
                ("C".into(), RfcValue::String(format!("Row{i}"))),
                ("STR".into(), RfcValue::String(format!("string {i}"))),
                ("XSTR".into(), RfcValue::Bytes(vec![0xAB; i as usize])),
            ])
        })
        .collect();
    IndexMap::from([("IMPORT_TAB".into(), RfcValue::Table(rows))])
}

fn read_table_inputs(where_clause: &str) -> IndexMap<String, RfcValue> {
    let option = IndexMap::from([("TEXT".into(), RfcValue::String(where_clause.into()))]);
    IndexMap::from([
        ("QUERY_TABLE".into(), RfcValue::String("DD03L".into())),
        ("DELIMITER".into(), RfcValue::String("|".into())),
        ("OPTIONS".into(), RfcValue::Table(vec![option])),
    ])
}

#[test]
#[ignore]
fn deep_table_chunks_equal_call() {
    let conn = connect();
    let rows = check_equal(
        &conn,
        "STFC_DEEP_TABLE",
        &deep_table_inputs(),
        "EXPORT_TAB",
        &[1, 2, 3, 100],
        &["RESPTEXT"],
    );
    assert_eq!(rows, 3);
}

#[test]
#[ignore]
fn read_table_chunks_equal_call() {
    let conn = connect();
    let inputs = read_table_inputs("TABNAME = 'T000'");
    let rows = check_equal(&conn, "RFC_READ_TABLE", &inputs, "DATA", &[1, 4, 1000], &[]);
    // The sizes above need a row count that 4 does not divide and that
    // 1000 exceeds.
    assert!(
        rows > 4 && !rows.is_multiple_of(4) && rows < 1000,
        "rows = {rows}"
    );
}

#[test]
#[ignore]
fn read_table_empty_table() {
    let conn = connect();
    let inputs = read_table_inputs("TABNAME = 'ZZZ_PYRFCX_NO_SUCH_TABLE'");
    let rows = check_equal(&conn, "RFC_READ_TABLE", &inputs, "DATA", &[1, 256], &[]);
    assert_eq!(rows, 0);
}

#[test]
#[ignore]
fn drop_half_way_then_call_again() {
    let conn = connect();
    let inputs = read_table_inputs("TABNAME = 'T000'");
    let expected = conn
        .call("RFC_READ_TABLE", &inputs, &[], CompatOptions::default())
        .expect("call");

    let mut outs = conn
        .call_chunked("RFC_READ_TABLE", &inputs, &[], CompatOptions::default())
        .expect("call_chunked");
    let Some(ChunkedOutput::Table(reader)) = outs.get_mut("DATA") else {
        panic!("DATA is not a table reader");
    };
    let total = reader.row_count();
    let first = reader.take_last_rows(2).expect("take_last_rows");
    assert_eq!(first.len(), 2);
    assert_eq!(reader.remaining(), total - 2);
    if let Some(RfcValue::Table(rows)) = expected.get("DATA") {
        assert_eq!(first[..], rows[total - 2..]);
    }

    // A reader moved to another thread keeps the container alive there.
    let Some(ChunkedOutput::Table(mut moved)) = outs.shift_remove("FIELDS") else {
        panic!("FIELDS is not a table reader");
    };
    drop(outs); // DATA dropped half-way
    let handle = std::thread::spawn(move || moved.take_last_rows(1).map(|c| c.len()));
    assert_eq!(handle.join().unwrap().expect("take on another thread"), 1);

    // The connection is still good for both kinds of call.
    let again = conn
        .call("RFC_READ_TABLE", &inputs, &[], CompatOptions::default())
        .expect("call after drop");
    assert_eq!(again, expected);
    let outs = conn
        .call_chunked("RFC_READ_TABLE", &inputs, &[], CompatOptions::default())
        .expect("call_chunked after drop");
    assert_eq!(assemble(outs, 3), expected);
}
