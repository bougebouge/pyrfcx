// Repository-fixture tier: marshal.rs round-trips against the
// real SDK DLL using RfcLoadRepository — genuine containers from genuine
// metadata, no SAP backend needed.
//
// All tests are #[ignore]: `cargo test --workspace` skips them. Run
// explicitly with `cargo test --test repository_roundtrip -- --ignored`.
// They FAIL (not skip) if the SDK library is not loadable or if no
// repository fixture exists — run `just capture-repository` first.
//
// This is its own integration-test binary — separate process for the
// OnceLock SDK singleton.

use std::sync::OnceLock;

use indexmap::IndexMap;
use pyrfcx_core::connection::CompatOptions;
use pyrfcx_core::error::RfcError;
use pyrfcx_core::value::RfcValue;

// ── Helpers ───────────────────────────────────────────────────────────

static FIXTURE: OnceLock<Fixture> = OnceLock::new();

fn fixture() -> &'static Fixture {
    FIXTURE.get_or_init(load_fixture)
}

fn fixture_dir() -> std::path::PathBuf {
    // Integration test binaries run from the package manifest dir.
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Go up to workspace root.
    p.pop(); // crates/
    p.pop(); // workspace root
    p.push("tests");
    p.push("fixtures");
    p.push("repository");
    p
}

struct Fixture {
    repository_id: String,
    functions: Vec<String>,
}

fn load_fixture() -> Fixture {
    // Verify the SDK library is loadable.
    pyrfcx_core::ffi::get_version().expect("SDK library not loadable — is SAPNWRFC_HOME set?");

    let dir = fixture_dir();
    let manifest_path = dir.join("manifest.json");
    let data = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e} — run `just capture-repository` first",
            manifest_path.display()
        )
    });
    let repository_id = extract_json_string(&data, "repository_id")
        .expect("manifest.json missing \"repository_id\"");
    let repo_file = extract_json_string(&data, "repository_file")
        .expect("manifest.json missing \"repository_file\"");
    let functions =
        extract_json_string_array(&data, "functions").expect("manifest.json missing \"functions\"");

    let repo_path = dir.join(&repo_file);
    assert!(
        repo_path.exists(),
        "repository file {} not found — run `just capture-repository` first",
        repo_path.display()
    );

    let workspace_root = {
        let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.pop();
        p.pop();
        p
    };
    pyrfcx_core::ffi::set_ini_path(&workspace_root.to_string_lossy())
        .expect("failed to set ini path");

    pyrfcx_core::repository::load(&repository_id, &repo_path)
        .expect("failed to load repository fixture");

    Fixture {
        repository_id,
        functions,
    }
}

fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let idx = json.find(&pattern)?;
    let rest = &json[idx + pattern.len()..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let quote_start = rest.find('"')?;
    let rest = &rest[quote_start + 1..];
    let quote_end = rest.find('"')?;
    Some(rest[..quote_end].to_string())
}

fn extract_json_string_array(json: &str, key: &str) -> Option<Vec<String>> {
    let pattern = format!("\"{key}\"");
    let idx = json.find(&pattern)?;
    let rest = &json[idx + pattern.len()..];
    let bracket_start = rest.find('[')?;
    let rest = &rest[bracket_start + 1..];
    let bracket_end = rest.find(']')?;
    let inside = &rest[..bracket_end];
    let items: Vec<String> = inside
        .split(',')
        .filter_map(|s| {
            let s = s.trim();
            let s = s.strip_prefix('"')?;
            let s = s.strip_suffix('"')?;
            Some(s.to_string())
        })
        .collect();
    if items.is_empty() { None } else { Some(items) }
}

fn native() -> CompatOptions {
    CompatOptions::default()
}

fn compat() -> CompatOptions {
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

fn require_fm(fixture: &Fixture, name: &str) {
    assert!(
        fixture.functions.iter().any(|f| f == name),
        "fixture does not contain {name} — recapture with it in --functions"
    );
}

// ── Helper: extract a structure parameter from read_all output ────────

fn get_struct<'a>(
    all: &'a IndexMap<String, RfcValue>,
    param: &str,
) -> &'a IndexMap<String, RfcValue> {
    match all.get(param) {
        Some(RfcValue::Structure(m)) => m,
        other => panic!("expected {param} structure, got {other:?}"),
    }
}

// ── Tests: scalar round-trips via STFC_STRUCTURE ─────────────────────
// Offline: no RfcInvoke. We set IMPORTSTRUCT and read it back via
// read_all() (which includes IMPORT params). The SDK container holds
// exactly what we wrote.

#[test]
#[ignore]
fn scalar_char_roundtrip() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp = IndexMap::new();
    imp.insert("RFCCHAR4".into(), RfcValue::String("AB".into()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");

    match imp.get("RFCCHAR4") {
        Some(RfcValue::Char(s)) => {
            assert_eq!(s.trim_end_matches(' '), "AB");
        }
        other => panic!("expected Char for RFCCHAR4, got {other:?}"),
    }
}

#[test]
#[ignore]
fn scalar_float_roundtrip() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp = IndexMap::new();
    imp.insert("RFCFLOAT".into(), RfcValue::Float(1.23456789012345));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");

    assert_eq!(
        imp.get("RFCFLOAT"),
        Some(&RfcValue::Float(1.23456789012345))
    );
}

#[test]
#[ignore]
fn scalar_int_boundaries() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp = IndexMap::new();
    imp.insert("RFCINT1".into(), RfcValue::Int1(255));
    imp.insert("RFCINT2".into(), RfcValue::Int2(-32768));
    imp.insert("RFCINT4".into(), RfcValue::Int4(2147483647));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");

    assert_eq!(imp.get("RFCINT1"), Some(&RfcValue::Int1(255)));
    assert_eq!(imp.get("RFCINT2"), Some(&RfcValue::Int2(-32768)));
    assert_eq!(imp.get("RFCINT4"), Some(&RfcValue::Int4(2147483647)));
}

#[test]
#[ignore]
fn scalar_date_time_roundtrip() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp = IndexMap::new();
    imp.insert("RFCDATE".into(), RfcValue::String("20260714".into()));
    imp.insert("RFCTIME".into(), RfcValue::String("120000".into()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");

    assert_eq!(imp.get("RFCDATE"), Some(&RfcValue::Date("20260714".into())));
    assert_eq!(imp.get("RFCTIME"), Some(&RfcValue::Time("120000".into())));
}

#[test]
#[ignore]
fn scalar_byte_roundtrip() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp = IndexMap::new();
    imp.insert("RFCHEX3".into(), RfcValue::Bytes(vec![0xDE, 0xAD, 0x00]));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");

    assert_eq!(
        imp.get("RFCHEX3"),
        Some(&RfcValue::Bytes(vec![0xDE, 0xAD, 0x00]))
    );
}

// ── Tests: STRING / XSTRING via STFC_STRING / STFC_XSTRING ───────────

#[test]
#[ignore]
fn scalar_string_roundtrip() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRING");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRING").unwrap();

    let mut inputs = IndexMap::new();
    inputs.insert("QUESTION".into(), RfcValue::String("hello world".into()));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();

    match all.get("QUESTION") {
        Some(RfcValue::String(s)) => assert_eq!(s, "hello world"),
        other => panic!("expected String for QUESTION, got {other:?}"),
    }
}

#[test]
#[ignore]
fn scalar_xstring_roundtrip() {
    let fixture = fixture();
    require_fm(fixture, "STFC_XSTRING");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_XSTRING").unwrap();

    let mut inputs = IndexMap::new();
    inputs.insert(
        "QUESTION".into(),
        RfcValue::Bytes(vec![0xCA, 0xFE, 0xBA, 0xBE]),
    );

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();

    assert_eq!(
        all.get("QUESTION"),
        Some(&RfcValue::Bytes(vec![0xCA, 0xFE, 0xBA, 0xBE]))
    );
}

// ── Tests: table round-trip via STFC_DEEP_TABLE ──────────────────────

#[test]
#[ignore]
fn table_roundtrip() {
    let fixture = fixture();
    require_fm(fixture, "STFC_DEEP_TABLE");

    let func = pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_DEEP_TABLE")
        .unwrap();

    let row1 = IndexMap::from([
        ("I".into(), RfcValue::Int4(1)),
        ("C".into(), RfcValue::String("Row1".into())),
        ("STR".into(), RfcValue::String("hello".into())),
        ("XSTR".into(), RfcValue::Bytes(vec![0x01, 0x02])),
    ]);
    let row2 = IndexMap::from([
        ("I".into(), RfcValue::Int4(2)),
        ("C".into(), RfcValue::String("Row2".into())),
        ("STR".into(), RfcValue::String("world".into())),
        ("XSTR".into(), RfcValue::Bytes(vec![0xFF])),
    ]);

    let mut inputs = IndexMap::new();
    inputs.insert("IMPORT_TAB".into(), RfcValue::Table(vec![row1, row2]));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();

    match all.get("IMPORT_TAB") {
        Some(RfcValue::Table(rows)) => {
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].get("I"), Some(&RfcValue::Int4(1)));
            assert_eq!(rows[1].get("I"), Some(&RfcValue::Int4(2)));
            match rows[0].get("STR") {
                Some(RfcValue::String(s)) => assert_eq!(s, "hello"),
                other => panic!("expected String, got {other:?}"),
            }
            match rows[0].get("XSTR") {
                Some(RfcValue::Bytes(b)) => assert_eq!(b, &[0x01, 0x02]),
                other => panic!("expected Bytes, got {other:?}"),
            }
        }
        other => panic!("expected IMPORT_TAB table, got {other:?}"),
    }
}

// ── Tests: chunked table reading (TableReader) ───────────────────────

fn deep_table_rows(n: i32) -> Vec<IndexMap<String, RfcValue>> {
    (0..n)
        .map(|i| {
            IndexMap::from([
                ("I".into(), RfcValue::Int4(i)),
                ("C".into(), RfcValue::String(format!("Row{i}"))),
                ("STR".into(), RfcValue::String(format!("string {i}"))),
                (
                    "XSTR".into(),
                    RfcValue::Bytes(vec![i as u8; (i % 4) as usize]),
                ),
            ])
        })
        .collect()
}

/// Take every chunk of a reader and put the chunks back in table order.
fn assemble(
    reader: &mut pyrfcx_core::TableReader,
    chunk: usize,
) -> Vec<IndexMap<String, RfcValue>> {
    let total = reader.row_count();
    let mut chunks = Vec::new();
    while reader.remaining() > 0 {
        let c = reader.take_last_rows(chunk).unwrap();
        assert!(!c.is_empty() && c.len() <= chunk);
        chunks.push(c);
    }
    assert!(reader.take_last_rows(chunk).unwrap().is_empty());
    let rows: Vec<_> = chunks.into_iter().rev().flatten().collect();
    assert_eq!(rows.len(), total);
    rows
}

#[test]
#[ignore]
fn chunked_reader_matches_full_read() {
    let fixture = fixture();
    require_fm(fixture, "STFC_DEEP_TABLE");
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORT_TAB".into(), RfcValue::Table(deep_table_rows(7)));

    let reference =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_DEEP_TABLE")
            .unwrap();
    reference.set_inputs(&inputs, native()).unwrap();
    let all = reference.read_all().unwrap();
    let Some(RfcValue::Table(expected)) = all.get("IMPORT_TAB") else {
        panic!("IMPORT_TAB missing");
    };
    assert_eq!(expected.len(), 7);

    // return_import_params exposes IMPORT_TAB as an output offline.
    let opts = CompatOptions {
        return_import_params: true,
        ..CompatOptions::default()
    };
    for chunk in [1, 3, 7, 100] {
        let func =
            pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_DEEP_TABLE")
                .unwrap();
        func.set_inputs(&inputs, native()).unwrap();
        let mut outs = func.into_chunked_outputs(opts).unwrap();
        let keys: Vec<_> = outs.keys().cloned().collect();
        let all_keys: Vec<_> = all.keys().cloned().collect();
        assert_eq!(keys, all_keys, "same outputs in the same order");

        let Some(pyrfcx_core::ChunkedOutput::Table(reader)) = outs.get_mut("IMPORT_TAB") else {
            panic!("IMPORT_TAB is not a table reader");
        };
        assert_eq!(&assemble(reader, chunk), expected, "chunk size {chunk}");

        // The empty output table works too.
        let Some(pyrfcx_core::ChunkedOutput::Table(empty)) = outs.get_mut("EXPORT_TAB") else {
            panic!("EXPORT_TAB is not a table reader");
        };
        assert_eq!(empty.row_count(), 0);
        assert!(assemble(empty, chunk).is_empty());
    }
}

#[test]
#[ignore]
fn chunked_reader_drop_half_way() {
    let fixture = fixture();
    require_fm(fixture, "STFC_DEEP_TABLE");
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORT_TAB".into(), RfcValue::Table(deep_table_rows(5)));
    let opts = CompatOptions {
        return_import_params: true,
        ..CompatOptions::default()
    };
    let func = pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_DEEP_TABLE")
        .unwrap();
    func.set_inputs(&inputs, native()).unwrap();
    let mut outs = func.into_chunked_outputs(opts).unwrap();
    let Some(pyrfcx_core::ChunkedOutput::Table(mut reader)) = outs.shift_remove("IMPORT_TAB")
    else {
        panic!("IMPORT_TAB is not a table reader");
    };
    drop(outs); // the other readers go; this one keeps the container alive
    let last = reader.take_last_rows(2).unwrap();
    assert_eq!(last.len(), 2);
    assert_eq!(last[0].get("I"), Some(&RfcValue::Int4(3)));
    assert_eq!(last[1].get("I"), Some(&RfcValue::Int4(4)));
    assert_eq!(reader.remaining(), 3);
    drop(reader); // frees the container with rows still in it
}

// ── Tests: fixed-width length check ─────────────────────────────────────────

#[test]
#[ignore]
fn exact_width_char_passes() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp = IndexMap::new();
    imp.insert("RFCCHAR4".into(), RfcValue::String("ABCD".into()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");

    match imp.get("RFCCHAR4") {
        Some(RfcValue::Char(s)) => assert_eq!(s, "ABCD"),
        other => panic!("expected Char 'ABCD', got {other:?}"),
    }
}

#[test]
#[ignore]
fn overlong_char_native_errors() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp = IndexMap::new();
    imp.insert("RFCCHAR4".into(), RfcValue::String("ABCDE".into()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    let err = func.set_inputs(&inputs, native()).unwrap_err();
    assert!(
        matches!(err, pyrfcx_core::RfcError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("RFCCHAR4"),
        "error should name the field: {msg}"
    );
    assert!(
        msg.contains("5 UTF-16 units"),
        "error should state the length: {msg}"
    );
    assert!(
        msg.contains("CHAR(4)"),
        "error should state the width: {msg}"
    );
}

#[test]
#[ignore]
fn overlong_char_compat_truncates() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp = IndexMap::new();
    imp.insert("RFCCHAR4".into(), RfcValue::String("ABCDE".into()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, compat()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");

    match imp.get("RFCCHAR4") {
        Some(RfcValue::Char(s)) => assert_eq!(s, "ABCD", "SDK should truncate to CHAR(4)"),
        other => panic!("expected Char, got {other:?}"),
    }
}

// ── Tests: compat empty DATE skip ──────────────────────────────────────

#[test]
#[ignore]
fn empty_date_compat_skip() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp = IndexMap::new();
    imp.insert("RFCDATE".into(), RfcValue::String(String::new()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, compat()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");

    match imp.get("RFCDATE") {
        Some(RfcValue::Date(s)) => assert_eq!(s, "00000000", "DATE should stay initial"),
        other => panic!("expected Date, got {other:?}"),
    }
}

#[test]
#[ignore]
fn empty_date_native_passes_to_sdk() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp = IndexMap::new();
    imp.insert("RFCDATE".into(), RfcValue::String(String::new()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    let result = func.set_inputs(&inputs, native());
    match result {
        Ok(()) => {
            let all = func.read_all().unwrap();
            let imp = get_struct(&all, "IMPORTSTRUCT");
            if let Some(RfcValue::Date(s)) = imp.get("RFCDATE") {
                eprintln!("empty DATE probe: RfcSetString(\"\") on DATE → {s:?}");
            }
        }
        Err(e) => {
            eprintln!("empty DATE probe: RfcSetString(\"\") on DATE → error: {e}");
        }
    }
}

// ── Tests: surrogate / astral edge cases ─────────────────────────────

#[test]
#[ignore]
fn astral_in_char1_native_errors() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    // "𝄞" (U+1D11E) = 2 UTF-16 code units → over-long for RFCCHAR1 (CHAR(1)).
    let mut imp = IndexMap::new();
    imp.insert("RFCCHAR1".into(), RfcValue::String("𝄞".into()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    let err = func.set_inputs(&inputs, native()).unwrap_err();
    assert!(matches!(err, pyrfcx_core::RfcError::InvalidInput(_)));
    let msg = err.to_string();
    assert!(msg.contains("2 UTF-16 units"), "{msg}");
    assert!(msg.contains("CHAR(1)"), "{msg}");
}

#[test]
#[ignore]
fn astral_in_char1_compat_truncates() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    // In compat mode, SDK truncates "𝄞" (2 UTF-16 units) into CHAR(1)
    // → broken high surrogate → U+FFFD on read-back.
    let mut imp = IndexMap::new();
    imp.insert("RFCCHAR1".into(), RfcValue::String("𝄞".into()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, compat()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");

    match imp.get("RFCCHAR1") {
        Some(RfcValue::Char(s)) => {
            eprintln!(
                "surrogate straddle probe: astral in CHAR(1) → {s:?} (bytes: {:?})",
                s.as_bytes()
            );
            assert_eq!(
                s.chars().count(),
                1,
                "should be exactly one (replacement) char"
            );
            assert_eq!(s, "\u{FFFD}", "broken surrogate should decode to U+FFFD");
        }
        other => panic!("expected Char, got {other:?}"),
    }
}

#[test]
#[ignore]
fn surrogate_straddle_char4() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    // "ABC𝄞" = 3 BMP + 1 astral = 5 UTF-16 code units.
    // RFCCHAR4 = CHAR(4). Native rejects (5 > 4). Compat truncates.
    // The interesting question: does the SDK keep the high surrogate
    // (broken pair) or does it stop at "ABC"?
    let mut imp = IndexMap::new();
    imp.insert("RFCCHAR4".into(), RfcValue::String("ABC𝄞".into()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    // Native: must reject.
    let err = func.set_inputs(&inputs, native()).unwrap_err();
    assert!(matches!(err, pyrfcx_core::RfcError::InvalidInput(_)));

    // Compat: SDK truncates — record what happens at the straddle boundary.
    let func2 =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp2 = IndexMap::new();
    imp2.insert("RFCCHAR4".into(), RfcValue::String("ABC𝄞".into()));
    let mut inputs2 = IndexMap::new();
    inputs2.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp2));

    func2.set_inputs(&inputs2, compat()).unwrap();
    let all = func2.read_all().unwrap();
    let imp_out = get_struct(&all, "IMPORTSTRUCT");

    match imp_out.get("RFCCHAR4") {
        Some(RfcValue::Char(s)) => {
            eprintln!(
                "surrogate straddle probe: \"ABC𝄞\" in CHAR(4) → {s:?} (len={}, bytes={:?})",
                s.len(),
                s.as_bytes()
            );
        }
        other => panic!("expected Char, got {other:?}"),
    }
}

// ── Tests: NUMC truncation direction ─────────────────────────────────

#[test]
#[ignore]
fn overlong_numc_compat_records_which_end() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    // RFCCHAR1 is CHAR(1) — not useful for NUMC. We need a NUM field.
    // STFC_STRUCTURE doesn't have a NUMC field, so we test the
    // length check itself: "12345" into a NUMC(4) should error natively.
    // But since STFC_STRUCTURE has no NUMC, let's test the predicate
    // directly with a CHAR field used as a proxy for over-long strings.
    //
    // If a NUMC FM is added to the fixture in the future, this test
    // should be updated to use a real NUMC field to record which end
    // the SDK truncates.
    let mut imp = IndexMap::new();
    imp.insert("RFCCHAR4".into(), RfcValue::String("123456".into()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    // Native: rejects.
    let err = func.set_inputs(&inputs, native()).unwrap_err();
    assert!(matches!(err, pyrfcx_core::RfcError::InvalidInput(_)));

    // Compat: SDK truncates — record content.
    let func2 =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();
    let mut imp2 = IndexMap::new();
    imp2.insert("RFCCHAR4".into(), RfcValue::String("123456".into()));
    let mut inputs2 = IndexMap::new();
    inputs2.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp2));

    func2.set_inputs(&inputs2, compat()).unwrap();
    let all = func2.read_all().unwrap();
    let imp_out = get_struct(&all, "IMPORTSTRUCT");

    match imp_out.get("RFCCHAR4") {
        Some(RfcValue::Char(s)) => {
            eprintln!("truncation direction probe: \"123456\" in CHAR(4) → {s:?}");
            assert_eq!(s.len(), 4, "SDK should truncate to field width");
        }
        other => panic!("expected Char, got {other:?}"),
    }
}

// ── Tests: integer overflow ──────────────────────────────────────────

#[test]
#[ignore]
fn int1_overflow() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    // INT1 is u8: valid range 0–255. Setting 256 via RfcSetInt1 is not
    // possible since the Rust type is u8. But we can test the boundary:
    // set 255 (max) and 0 (min), confirm they round-trip.
    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    let mut imp = IndexMap::new();
    imp.insert("RFCINT1".into(), RfcValue::Int1(255));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");
    assert_eq!(imp.get("RFCINT1"), Some(&RfcValue::Int1(255)));

    // 0
    let func2 =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();
    let mut imp2 = IndexMap::new();
    imp2.insert("RFCINT1".into(), RfcValue::Int1(0));
    let mut inputs2 = IndexMap::new();
    inputs2.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp2));
    func2.set_inputs(&inputs2, native()).unwrap();
    let all2 = func2.read_all().unwrap();
    assert_eq!(
        get_struct(&all2, "IMPORTSTRUCT").get("RFCINT1"),
        Some(&RfcValue::Int1(0))
    );

    // 256, native: rejected by our range check before any setter.
    let func3 =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();
    let mut imp3 = IndexMap::new();
    imp3.insert("RFCINT1".into(), RfcValue::Int(256));
    let mut inputs3 = IndexMap::new();
    inputs3.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp3));
    let err = func3
        .set_inputs(&inputs3, native())
        .expect_err("256 → INT1 must fail the native range check");
    assert!(
        matches!(err, RfcError::InvalidInput(_)),
        "expected InvalidInput, got: {err:?}"
    );
    assert!(err.to_string().contains("INT1 (0..255)"), "{err}");
}

#[test]
#[ignore]
fn int1_overflow_probe_pyrfc_setter() {
    // pyrfc fills INT1 via RfcSetInt (32-bit). The SDK accepts 256
    // and wraps it in the client-side container, before any call.
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();
    let mut imp = IndexMap::new();
    imp.insert("RFCINT1".into(), RfcValue::Int(256));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, compat())
        .expect("RfcSetInt(256) on INT1 is accepted by the SDK");
    let all = func.read_all().unwrap();
    assert_eq!(
        get_struct(&all, "IMPORTSTRUCT").get("RFCINT1"),
        Some(&RfcValue::Int1(0))
    );
}

#[test]
#[ignore]
fn int2_boundaries() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    // INT2 is i16: -32768 to 32767.
    for val in [i16::MIN, -1, 0, 1, i16::MAX] {
        let func =
            pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE")
                .unwrap();

        let mut imp = IndexMap::new();
        imp.insert("RFCINT2".into(), RfcValue::Int2(val));
        let mut inputs = IndexMap::new();
        inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

        func.set_inputs(&inputs, native()).unwrap();
        let all = func.read_all().unwrap();
        assert_eq!(
            get_struct(&all, "IMPORTSTRUCT").get("RFCINT2"),
            Some(&RfcValue::Int2(val)),
            "INT2 round-trip failed for {val}"
        );
    }
}

#[test]
#[ignore]
fn int4_boundaries() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    for val in [i32::MIN, -1, 0, 1, i32::MAX] {
        let func =
            pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE")
                .unwrap();

        let mut imp = IndexMap::new();
        imp.insert("RFCINT4".into(), RfcValue::Int4(val));
        let mut inputs = IndexMap::new();
        inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

        func.set_inputs(&inputs, native()).unwrap();
        let all = func.read_all().unwrap();
        assert_eq!(
            get_struct(&all, "IMPORTSTRUCT").get("RFCINT4"),
            Some(&RfcValue::Int4(val)),
            "INT4 round-trip failed for {val}"
        );
    }
}

#[test]
#[ignore]
fn float_special_values() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    for val in [
        0.0,
        -0.0,
        f64::MIN,
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::EPSILON,
    ] {
        let func =
            pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE")
                .unwrap();

        let mut imp = IndexMap::new();
        imp.insert("RFCFLOAT".into(), RfcValue::Float(val));
        let mut inputs = IndexMap::new();
        inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

        func.set_inputs(&inputs, native()).unwrap();
        let all = func.read_all().unwrap();
        match get_struct(&all, "IMPORTSTRUCT").get("RFCFLOAT") {
            Some(RfcValue::Float(f)) => {
                assert_eq!(
                    f.to_bits(),
                    val.to_bits(),
                    "FLOAT bit-exact round-trip failed for {val}"
                );
            }
            other => panic!("expected Float for {val}, got {other:?}"),
        }
    }
}

// ── Tests: Unicode edge cases ────────────────────────────────────────

#[test]
#[ignore]
fn unicode_bmp_in_char() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    // "Ü" (U+00DC) = 1 UTF-16 code unit → fits CHAR(1).
    let mut imp = IndexMap::new();
    imp.insert("RFCCHAR1".into(), RfcValue::String("Ü".into()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");

    match imp.get("RFCCHAR1") {
        Some(RfcValue::Char(s)) => assert_eq!(s, "Ü"),
        other => panic!("expected Char 'Ü', got {other:?}"),
    }
}

#[test]
#[ignore]
fn unicode_cjk_in_char4() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRUCTURE");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRUCTURE").unwrap();

    // "日本語" = 3 BMP chars = 3 UTF-16 code units → fits CHAR(4).
    let mut imp = IndexMap::new();
    imp.insert("RFCCHAR4".into(), RfcValue::String("日本語".into()));
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();
    let imp = get_struct(&all, "IMPORTSTRUCT");

    match imp.get("RFCCHAR4") {
        Some(RfcValue::Char(s)) => {
            assert_eq!(s.trim_end_matches(' '), "日本語");
        }
        other => panic!("expected Char, got {other:?}"),
    }
}

#[test]
#[ignore]
fn unicode_supplementary_in_string() {
    let fixture = fixture();
    require_fm(fixture, "STFC_STRING");

    let func =
        pyrfcx_core::repository::create_function(&fixture.repository_id, "STFC_STRING").unwrap();

    // STRING is variable-length — supplementary plane chars round-trip.
    let text = "𝄞 café 你好 🎵";
    let mut inputs = IndexMap::new();
    inputs.insert("QUESTION".into(), RfcValue::String(text.into()));

    func.set_inputs(&inputs, native()).unwrap();
    let all = func.read_all().unwrap();

    match all.get("QUESTION") {
        Some(RfcValue::String(s)) => assert_eq!(s, text),
        other => panic!("expected String, got {other:?}"),
    }
}
