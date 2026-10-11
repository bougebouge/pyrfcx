// Live tests for function descriptions and RfcIsConnectionHandleValid
// against SAP destination TEST_NW750.
//
// All tests are #[ignore] — run with: cargo test -- --ignored

use std::sync::Once;

use pyrfcx_core::{Connection, ConnectionParams, RfcDirection, RfcFieldType, TypeInfo};

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

/// The SDK descriptor's names, lengths, offsets, texts, defaults and the
/// optional flag, on a structure FM and on our own echo FM.
#[test]
#[ignore]
fn describe_function_reads_the_full_descriptor() {
    let conn = connect();

    let desc = conn.describe_function("STFC_STRUCTURE").unwrap();
    assert_eq!(desc.name, "STFC_STRUCTURE");
    let names: Vec<&str> = desc.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["ECHOSTRUCT", "RESPTEXT", "IMPORTSTRUCT", "RFCTABLE"]
    );

    let resptext = desc.param("RESPTEXT").unwrap();
    assert_eq!(resptext.direction, RfcDirection::Export);
    assert_eq!(resptext.text, "Exporting response message");
    assert_eq!(
        (resptext.desc.nuc_length, resptext.desc.uc_length),
        (255, 510)
    );
    assert!(resptext.desc.type_info.is_none() && resptext.desc.fields.is_empty());

    let table = desc.param("RFCTABLE").unwrap();
    assert_eq!(table.desc.rfctype, RfcFieldType::Table);
    assert_eq!(
        table.desc.type_info,
        Some(TypeInfo {
            name: "RFCTEST".into(),
            nuc_length: 144,
            uc_length: 264,
        })
    );
    let char4 = table.desc.field("RFCCHAR4").unwrap();
    assert_eq!((char4.nuc_offset, char4.uc_offset), (13, 14));
    assert_eq!((char4.desc.nuc_length, char4.desc.uc_length), (4, 8));

    let desc = conn.describe_function("ZPYRFCX_FE_DATATYPES").unwrap();
    let count = desc.param("IV_COUNT").unwrap();
    assert_eq!(count.default_value, "0");
    assert!(count.optional);
    assert!(!desc.param("ES_OUTPUT").unwrap().optional);
}

#[test]
#[ignore]
fn is_valid_until_closed() {
    let mut conn = connect();
    assert!(conn.is_valid().unwrap());
    conn.close().unwrap();
    assert!(!conn.is_valid().unwrap());
}
