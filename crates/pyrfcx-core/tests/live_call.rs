// Live integration tests for Connection::call() against SAP destination TEST_NW750.
//
// All tests are #[ignore] — run with: cargo test -- --ignored
//
// These test at the RfcValue level, asserting exact variant discrimination
// (the lossless property the plan requires). Python tests verify the same
// scenarios after PyO3 output conversion; these verify before it.

use std::sync::Once;

use indexmap::IndexMap;
use pyrfcx_core::{CompatOptions, Connection, ConnectionParams, RfcError, RfcValue};

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

fn call(
    conn: &Connection,
    fm: &str,
    inputs: IndexMap<String, RfcValue>,
) -> IndexMap<String, RfcValue> {
    conn.call(fm, &inputs, &[], CompatOptions::default())
        .unwrap_or_else(|e| panic!("{fm} failed: {e}"))
}

fn call_err(conn: &Connection, fm: &str, inputs: IndexMap<String, RfcValue>) -> RfcError {
    conn.call(fm, &inputs, &[], CompatOptions::default())
        .expect_err(&format!("{fm} should have failed"))
}

fn stfc_structure(
    conn: &Connection,
    imp: IndexMap<String, RfcValue>,
) -> IndexMap<String, RfcValue> {
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));
    call(conn, "STFC_STRUCTURE", inputs)
}

fn echo(result: &IndexMap<String, RfcValue>) -> &IndexMap<String, RfcValue> {
    match result.get("ECHOSTRUCT").expect("missing ECHOSTRUCT") {
        RfcValue::Structure(m) => m,
        other => panic!("ECHOSTRUCT is not a Structure: {other:?}"),
    }
}

fn table_rows(result: &IndexMap<String, RfcValue>) -> &Vec<IndexMap<String, RfcValue>> {
    match result.get("RFCTABLE").expect("missing RFCTABLE") {
        RfcValue::Table(rows) => rows,
        other => panic!("RFCTABLE is not a Table: {other:?}"),
    }
}

// ── Scalar echo round-trip ─────────────────────────────────────────────

#[test]
#[ignore]
fn echo_scalars() {
    let conn = connect();
    let imp = IndexMap::from([
        ("RFCFLOAT".into(), RfcValue::Float(3.125)),
        ("RFCINT4".into(), RfcValue::Int(42)),
        ("RFCINT2".into(), RfcValue::Int(-1)),
        ("RFCINT1".into(), RfcValue::Int(7)),
        ("RFCCHAR1".into(), RfcValue::String("A".into())),
        ("RFCCHAR2".into(), RfcValue::String("BC".into())),
        ("RFCCHAR4".into(), RfcValue::String("DEFG".into())),
        ("RFCDATE".into(), RfcValue::String("20260714".into())),
        ("RFCTIME".into(), RfcValue::String("120000".into())),
        ("RFCHEX3".into(), RfcValue::Bytes(vec![0xDE, 0xAD, 0x00])),
    ]);
    let result = stfc_structure(&conn, imp);
    let e = echo(&result);

    // Lossless: each field comes back as the exact RfcValue variant
    // matching its RFCTYPE, not a generic String.
    assert!(
        matches!(e.get("RFCFLOAT"), Some(RfcValue::Float(f)) if (*f - 3.125).abs() < f64::EPSILON)
    );
    assert!(matches!(e.get("RFCINT4"), Some(RfcValue::Int4(42))));
    assert!(matches!(e.get("RFCINT2"), Some(RfcValue::Int2(-1))));
    assert!(matches!(e.get("RFCINT1"), Some(RfcValue::Int1(7))));
    assert!(matches!(e.get("RFCDATE"), Some(RfcValue::Date(d)) if d == "20260714"));
    assert!(matches!(e.get("RFCTIME"), Some(RfcValue::Time(t)) if t == "120000"));
    assert!(matches!(e.get("RFCHEX3"), Some(RfcValue::Bytes(b)) if b == &[0xDE, 0xAD, 0x00]));

    // CHAR fields come back as RfcValue::Char — full padded value.
    if let Some(RfcValue::Char(s)) = e.get("RFCCHAR1") {
        assert_eq!(s.trim_end_matches(' '), "A");
    } else {
        panic!("RFCCHAR1 should be Char, got: {:?}", e.get("RFCCHAR1"));
    }
    if let Some(RfcValue::Char(s)) = e.get("RFCCHAR4") {
        assert_eq!(s.trim_end_matches(' '), "DEFG");
    } else {
        panic!("RFCCHAR4 should be Char, got: {:?}", e.get("RFCCHAR4"));
    }
}

// ── Unicode ────────────────────────────────────────────────────────────

#[test]
#[ignore]
fn echo_unicode_bmp() {
    let conn = connect();
    let imp = IndexMap::from([("RFCCHAR4".into(), RfcValue::String("Ü日".into()))]);
    let result = stfc_structure(&conn, imp);
    let e = echo(&result);
    if let Some(RfcValue::Char(s)) = e.get("RFCCHAR4") {
        assert_eq!(s.trim_end_matches(' '), "Ü日");
    } else {
        panic!("RFCCHAR4 should be Char, got: {:?}", e.get("RFCCHAR4"));
    }
}

#[test]
#[ignore]
fn echo_unicode_supplementary() {
    let conn = connect();
    let imp = IndexMap::from([("RFCDATA1".into(), RfcValue::String("𝄞𠀀".into()))]);
    let result = stfc_structure(&conn, imp);
    let e = echo(&result);
    if let Some(RfcValue::Char(s)) = e.get("RFCDATA1") {
        assert_eq!(s.trim_end_matches(' '), "𝄞𠀀");
    } else {
        panic!("RFCDATA1 should be Char, got: {:?}", e.get("RFCDATA1"));
    }
}

// ── Table output ───────────────────────────────────────────────────────

#[test]
#[ignore]
fn table_has_mutated_row() {
    let conn = connect();
    let imp = IndexMap::from([
        ("RFCFLOAT".into(), RfcValue::Float(10.0)),
        ("RFCINT4".into(), RfcValue::Int(100)),
    ]);
    let result = stfc_structure(&conn, imp);
    let rows = table_rows(&result);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert!(
        matches!(row.get("RFCFLOAT"), Some(RfcValue::Float(f)) if (*f - 11.0).abs() < f64::EPSILON)
    );
    assert!(matches!(row.get("RFCINT4"), Some(RfcValue::Int4(101))));
    assert!(matches!(row.get("RFCHEX3"), Some(RfcValue::Bytes(b)) if b == &[0xF1, 0xF2, 0xF3]));
}

// ── Float precision ────────────────────────────────────────────────────

#[test]
#[ignore]
fn float_precision() {
    let conn = connect();
    let imp = IndexMap::from([("RFCFLOAT".into(), RfcValue::Float(1.23456789012345))]);
    let result = stfc_structure(&conn, imp);
    let e = echo(&result);
    assert!(
        matches!(e.get("RFCFLOAT"), Some(RfcValue::Float(f)) if (*f - 1.23456789012345).abs() < 1e-14)
    );
}

// ── Int boundaries ─────────────────────────────────────────────────────

#[test]
#[ignore]
fn int_boundaries() {
    let conn = connect();
    let imp = IndexMap::from([
        ("RFCINT1".into(), RfcValue::Int(0)),
        ("RFCINT2".into(), RfcValue::Int(-32768)),
        ("RFCINT4".into(), RfcValue::Int(2147483646)),
    ]);
    let result = stfc_structure(&conn, imp);
    let e = echo(&result);
    assert!(matches!(e.get("RFCINT1"), Some(RfcValue::Int1(0))));
    assert!(matches!(e.get("RFCINT2"), Some(RfcValue::Int2(-32768))));
    assert!(matches!(e.get("RFCINT4"), Some(RfcValue::Int4(2147483646))));
}

// ── Date/time edge cases ───────────────────────────────────────────────

#[test]
#[ignore]
fn initial_date() {
    let conn = connect();
    let imp = IndexMap::from([("RFCDATE".into(), RfcValue::String("00000000".into()))]);
    let result = stfc_structure(&conn, imp);
    let e = echo(&result);
    assert!(matches!(e.get("RFCDATE"), Some(RfcValue::Date(d)) if d == "00000000"));
}

#[test]
#[ignore]
fn invalid_date_preserved() {
    let conn = connect();
    let imp = IndexMap::from([("RFCDATE".into(), RfcValue::String("99999999".into()))]);
    let result = stfc_structure(&conn, imp);
    let e = echo(&result);
    assert!(matches!(e.get("RFCDATE"), Some(RfcValue::Date(d)) if d == "99999999"));
}

#[test]
#[ignore]
fn empty_date_sent_as_initial() {
    // Native sends an empty DATE as the initial value, so every SDK patch
    // level gives the same answer (compat skips the setter instead).
    // Structure field and table row both go through the field setter.
    let conn = connect();
    let imp = IndexMap::from([("RFCDATE".into(), RfcValue::String(String::new()))]);
    let row = IndexMap::from([("RFCDATE".into(), RfcValue::String(String::new()))]);
    let inputs = IndexMap::from([
        ("IMPORTSTRUCT".to_string(), RfcValue::Structure(imp)),
        ("RFCTABLE".to_string(), RfcValue::Table(vec![row])),
    ]);
    let result = call(&conn, "STFC_STRUCTURE", inputs);
    let e = echo(&result);
    assert!(matches!(e.get("RFCDATE"), Some(RfcValue::Date(d)) if d == "00000000"));
    let rows = table_rows(&result);
    assert!(matches!(rows[0].get("RFCDATE"), Some(RfcValue::Date(d)) if d == "00000000"));
}

#[test]
#[ignore]
fn empty_time_sent_as_initial() {
    // As for DATE above.
    let conn = connect();
    let imp = IndexMap::from([("RFCTIME".into(), RfcValue::String(String::new()))]);
    let result = stfc_structure(&conn, imp);
    let e = echo(&result);
    assert!(matches!(e.get("RFCTIME"), Some(RfcValue::Time(t)) if t == "000000"));
}

// ── STFC_DEEP_TABLE ────────────────────────────────────────────────────
// EXPORT_TAB = copy of IMPORT_TAB + one appended row
//   {I: 10, C: "Appended", STR: SY-DATUM, XSTR: X'DEADBEEF'}.
// Tests STRING and XSTRING (variable-length) types.

#[test]
#[ignore]
fn deep_table_echo_and_appended_row() {
    let conn = connect();
    let row1 = IndexMap::from([
        ("I".into(), RfcValue::Int(1)),
        ("C".into(), RfcValue::String("Row1".into())),
        ("STR".into(), RfcValue::String("hello".into())),
        ("XSTR".into(), RfcValue::Bytes(vec![0x01, 0x02])),
    ]);
    let row2 = IndexMap::from([
        ("I".into(), RfcValue::Int(2)),
        ("C".into(), RfcValue::String("Row2".into())),
        ("STR".into(), RfcValue::String("world".into())),
        ("XSTR".into(), RfcValue::Bytes(vec![0xFF])),
    ]);
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORT_TAB".into(), RfcValue::Table(vec![row1, row2]));
    let result = call(&conn, "STFC_DEEP_TABLE", inputs);

    let tab = match result.get("EXPORT_TAB").unwrap() {
        RfcValue::Table(rows) => rows,
        other => panic!("EXPORT_TAB should be Table, got: {other:?}"),
    };
    assert_eq!(tab.len(), 3);

    // Row 0: echoed verbatim
    assert!(matches!(tab[0].get("I"), Some(RfcValue::Int4(1))));
    if let Some(RfcValue::Char(s)) = tab[0].get("C") {
        assert_eq!(s.trim_end_matches(' '), "Row1");
    } else {
        panic!("C should be Char, got: {:?}", tab[0].get("C"));
    }
    assert!(matches!(tab[0].get("STR"), Some(RfcValue::String(s)) if s == "hello"));
    assert!(matches!(tab[0].get("XSTR"), Some(RfcValue::Bytes(b)) if b == &[0x01, 0x02]));

    // Row 1: echoed verbatim
    assert!(matches!(tab[1].get("I"), Some(RfcValue::Int4(2))));
    assert!(matches!(tab[1].get("STR"), Some(RfcValue::String(s)) if s == "world"));
    assert!(matches!(tab[1].get("XSTR"), Some(RfcValue::Bytes(b)) if b == &[0xFF]));

    // Row 2: appended by ABAP
    assert!(matches!(tab[2].get("I"), Some(RfcValue::Int4(10))));
    if let Some(RfcValue::Char(s)) = tab[2].get("C") {
        assert_eq!(s.trim_end_matches(' '), "Appended");
    } else {
        panic!("C should be Char, got: {:?}", tab[2].get("C"));
    }
    assert!(
        matches!(tab[2].get("XSTR"), Some(RfcValue::Bytes(b)) if b == &[0xDE, 0xAD, 0xBE, 0xEF])
    );
}

#[test]
#[ignore]
fn deep_table_empty_input() {
    let conn = connect();
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORT_TAB".into(), RfcValue::Table(vec![]));
    let result = call(&conn, "STFC_DEEP_TABLE", inputs);

    let tab = match result.get("EXPORT_TAB").unwrap() {
        RfcValue::Table(rows) => rows,
        other => panic!("EXPORT_TAB should be Table, got: {other:?}"),
    };
    assert_eq!(tab.len(), 1);
    assert!(matches!(tab[0].get("I"), Some(RfcValue::Int4(10))));
}

#[test]
#[ignore]
fn deep_table_unicode_in_string_field() {
    let conn = connect();
    let row = IndexMap::from([
        ("I".into(), RfcValue::Int(0)),
        ("C".into(), RfcValue::String(String::new())),
        ("STR".into(), RfcValue::String("𝄞 café 你好".into())),
        ("XSTR".into(), RfcValue::Bytes(vec![])),
    ]);
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORT_TAB".into(), RfcValue::Table(vec![row]));
    let result = call(&conn, "STFC_DEEP_TABLE", inputs);

    let tab = match result.get("EXPORT_TAB").unwrap() {
        RfcValue::Table(rows) => rows,
        other => panic!("EXPORT_TAB should be Table, got: {other:?}"),
    };
    // STRING is variable-length — no padding.
    assert!(matches!(tab[0].get("STR"), Some(RfcValue::String(s)) if s == "𝄞 café 你好"));
}

// ── Edge cases ─────────────────────────────────────────────────────────

#[test]
#[ignore]
fn unknown_parameter_name() {
    let conn = connect();
    let mut inputs = IndexMap::new();
    inputs.insert(
        "IMPORTSTRUCT".into(),
        RfcValue::Structure(IndexMap::from([("RFCFLOAT".into(), RfcValue::Float(0.0))])),
    );
    inputs.insert("DOESNOTEXIST".into(), RfcValue::String("x".into()));
    let err = call_err(&conn, "STFC_STRUCTURE", inputs);
    let msg = err.to_string();
    assert!(
        !msg.is_empty(),
        "should produce an error for unknown parameter"
    );
}

#[test]
#[ignore]
fn set_export_only_parameter() {
    let conn = connect();
    let mut inputs = IndexMap::new();
    inputs.insert(
        "ECHOSTRUCT".into(),
        RfcValue::Structure(IndexMap::from([("RFCFLOAT".into(), RfcValue::Float(1.0))])),
    );
    let err = call_err(&conn, "STFC_STRUCTURE", inputs);
    assert!(
        matches!(err, RfcError::InvalidInput(_)),
        "expected InvalidInput, got: {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("EXPORT-only"),
        "should mention EXPORT-only: {msg}"
    );
}

#[test]
#[ignore]
fn multiple_calls_same_connection() {
    let conn = connect();
    for i in 0..5 {
        let imp = IndexMap::from([("RFCINT4".into(), RfcValue::Int(i128::from(i)))]);
        let result = stfc_structure(&conn, imp);
        let e = echo(&result);
        assert!(matches!(e.get("RFCINT4"), Some(RfcValue::Int4(v)) if *v == i));
    }
}

#[test]
#[ignore]
fn char50_overflow_native_errors() {
    let conn = connect();
    let long_text = "A".repeat(100);
    let imp = IndexMap::from([("RFCDATA1".into(), RfcValue::String(long_text))]);
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));
    let err = call_err(&conn, "STFC_STRUCTURE", inputs);
    assert!(
        matches!(err, RfcError::InvalidInput(_)),
        "expected InvalidInput, got: {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("RFCDATA1"),
        "error should name the field: {msg}"
    );
    assert!(
        msg.contains("CHAR(50)"),
        "error should name the width: {msg}"
    );
}

#[test]
#[ignore]
fn char50_overflow_compat_truncates() {
    let conn = connect();
    let long_text = "A".repeat(100);
    let imp = IndexMap::from([("RFCDATA1".into(), RfcValue::String(long_text))]);
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));
    let compat = CompatOptions {
        skip_empty_date_time_inputs: true,
        skip_input_length_check: true,
        pyrfc_int_setters: true,
        skip_decimal_input_check: true,
        deactivate_before_inputs: true,
        read_inactive_outputs: true,
        pyrfc_date_time_setters: true,
        return_import_params: false,
    };
    let result = conn
        .call("STFC_STRUCTURE", &inputs, &[], compat)
        .expect("compat call should succeed");
    let e = echo(&result);
    if let Some(RfcValue::Char(s)) = e.get("RFCDATA1") {
        assert_eq!(s.trim_end_matches(' '), &"A".repeat(50));
    } else {
        panic!("RFCDATA1 should be Char, got: {:?}", e.get("RFCDATA1"));
    }
}

// ── integer input dispatch ───────────────────────────────────────

#[test]
#[ignore]
fn string_input_rejected_for_int_field() {
    // Strings no longer convert to integer targets — InvalidType
    // before any SDK call.
    let conn = connect();
    let imp = IndexMap::from([("RFCINT4".into(), RfcValue::String("42".into()))]);
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));
    let err = call_err(&conn, "STFC_STRUCTURE", inputs);
    assert!(
        matches!(err, RfcError::InvalidType(_)),
        "expected InvalidType, got: {err:?}"
    );
    let msg = err.to_string();
    assert!(msg.contains("RFCINT4"), "{msg}");
    assert!(msg.contains("INT4 field requires int input"), "{msg}");
}

#[test]
#[ignore]
fn int_out_of_range_native_errors() {
    // 256 → INT1: native range check fires before any setter.
    let conn = connect();
    let imp = IndexMap::from([("RFCINT1".into(), RfcValue::Int(256))]);
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));
    let err = call_err(&conn, "STFC_STRUCTURE", inputs);
    assert!(
        matches!(err, RfcError::InvalidInput(_)),
        "expected InvalidInput, got: {err:?}"
    );
    let msg = err.to_string();
    assert!(msg.contains("RFCINT1"), "{msg}");
    assert!(msg.contains("INT1 (0..255)"), "{msg}");
}

#[test]
#[ignore]
fn int_out_of_range_compat_delegates_to_sdk() {
    // Compat: 256 → INT1 goes through RfcSetInt (pyrfc's setter)
    // and the SDK judges it. This test records the SDK's verdict — the
    // The setter probe pins the exact error for the compat template.
    let conn = connect();
    let imp = IndexMap::from([("RFCINT1".into(), RfcValue::Int(256))]);
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));
    let compat = CompatOptions {
        skip_empty_date_time_inputs: true,
        skip_input_length_check: true,
        pyrfc_int_setters: true,
        skip_decimal_input_check: true,
        deactivate_before_inputs: true,
        read_inactive_outputs: true,
        pyrfc_date_time_setters: true,
        return_import_params: false,
    };
    let result = conn.call("STFC_STRUCTURE", &inputs, &[], compat);
    // Whatever the SDK does (error or narrow), it must NOT be our
    // client-side InvalidInput/InvalidType.
    if let Err(e) = result {
        assert!(
            !matches!(e, RfcError::InvalidInput(_) | RfcError::InvalidType(_)),
            "compat must delegate to the SDK, got client-side: {e:?}"
        );
    }
}

#[test]
#[ignore]
fn int_input_rejected_for_char_field() {
    // Int is only valid for numeric targets — CHAR rejects it
    // with InvalidType (STFC_STRUCTURE has no NUMC field to assert the
    // accept side; the repository fixtures cover it offline).
    let conn = connect();
    let imp = IndexMap::from([("RFCCHAR4".into(), RfcValue::Int(42))]);
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));
    let err = call_err(&conn, "STFC_STRUCTURE", inputs);
    assert!(
        matches!(err, RfcError::InvalidType(_)),
        "expected InvalidType, got: {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("int input is not valid for CHAR field"),
        "{msg}"
    );
}

// ── Error handling ─────────────────────────────────────────────────────

#[test]
#[ignore]
fn nonexistent_fm() {
    let conn = connect();
    let err = call_err(&conn, "DOES_NOT_EXIST_ZZZZZ", IndexMap::new());
    if let RfcError::AbapException(info) = &err {
        assert_eq!(info.key, "FU_NOT_FOUND");
    } else {
        panic!("expected AbapException, got: {err:?}");
    }
}

#[test]
#[ignore]
fn connection_alive_after_abap_exception() {
    let conn = connect();
    let err = call_err(&conn, "DOES_NOT_EXIST_ZZZZZ", IndexMap::new());
    assert!(matches!(err, RfcError::AbapException(_)));

    // Connection should still work.
    let result = call(&conn, "RFC_PING", IndexMap::new());
    assert!(result.is_empty() || !result.is_empty()); // just didn't panic
}

#[test]
#[ignore]
fn int1_overflow_kills_connection() {
    let conn = connect();
    let imp = IndexMap::from([("RFCINT1".into(), RfcValue::Int(255))]);
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));
    let err = call_err(&conn, "STFC_STRUCTURE", inputs);
    assert!(
        matches!(err, RfcError::AbapRuntimeFailure(_)),
        "expected AbapRuntimeFailure, got: {err:?}"
    );
}

// ── RFC_PING ───────────────────────────────────────────────────────────

#[test]
#[ignore]
fn ping() {
    let conn = connect();
    let result = call(&conn, "RFC_PING", IndexMap::new());
    // RFC_PING has no output parameters.
    assert!(result.is_empty());
}

// ── RESPTEXT ───────────────────────────────────────────────────────────

#[test]
#[ignore]
fn resptext_contains_sysid() {
    let conn = connect();
    let imp = IndexMap::from([("RFCFLOAT".into(), RfcValue::Float(0.0))]);
    let result = stfc_structure(&conn, imp);
    if let Some(RfcValue::Char(s)) = result.get("RESPTEXT") {
        assert!(s.contains("Sysid:"), "RESPTEXT should contain Sysid: {s}");
    } else {
        panic!("RESPTEXT should be Char, got: {:?}", result.get("RESPTEXT"));
    }
}

// ── NUMC over-long input ───────────────────────
//
// STFC_STRUCTURE has no NUMC field, so this probe uses our own module.
// ZPYRFCX_FE_DATATYPES.ZNUMC is NUMC(10). Result: NUMC has no truncation
// direction — the SDK REJECTS over-long input instead of truncating it,
// unlike CHAR. True in compat mode too, so the native length check is not
// what shields compat here; the SDK does.
#[test]
#[ignore]
fn overlong_numc_is_rejected_by_the_sdk_even_in_compat() {
    let conn = connect();
    let imp = IndexMap::from([(
        "ZNUMC".into(),
        RfcValue::String("12345678901234".into()), // 14 digits into NUMC(10)
    )]);
    let mut inputs = IndexMap::new();
    inputs.insert("IS_INPUT".into(), RfcValue::Structure(imp));
    let compat = CompatOptions {
        skip_empty_date_time_inputs: true,
        skip_input_length_check: true,
        pyrfc_int_setters: true,
        skip_decimal_input_check: true,
        deactivate_before_inputs: true,
        read_inactive_outputs: true,
        pyrfc_date_time_setters: true,
        return_import_params: false,
    };
    let err = conn
        .call("ZPYRFCX_FE_DATATYPES", &inputs, &[], compat)
        .expect_err("over-long NUMC must not be accepted");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("RFC_CONVERSION_FAILURE") && msg.contains("RFCTYPE_NUM"),
        "expected an SDK conversion failure, got: {msg}"
    );
}
