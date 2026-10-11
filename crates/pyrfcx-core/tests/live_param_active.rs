// Live tests + probes for parameter deactivation (RfcSetParameterActive)
// against SAP destination TEST_NW750.
//
// All tests are #[ignore] — run with: cargo test -- --ignored

use indexmap::IndexMap;

use pyrfcx_core::{CompatOptions, Connection, ConnectionParams, RfcValue};

use std::sync::Once;

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
fn inactive_output_is_omitted_from_result() {
    let conn = connect();
    let imp = IndexMap::from([("RFCDATA1".into(), RfcValue::String("payload".into()))]);
    let mut inputs = IndexMap::new();
    inputs.insert("IMPORTSTRUCT".into(), RfcValue::Structure(imp));

    // Baseline: ECHOSTRUCT is returned when active.
    let full = conn
        .call("STFC_STRUCTURE", &inputs, &[], CompatOptions::default())
        .expect("baseline call failed");
    assert!(full.contains_key("ECHOSTRUCT"));
    assert!(full.contains_key("RESPTEXT"));

    // Deactivated: ECHOSTRUCT must be omitted, the rest untouched.
    let trimmed = conn
        .call(
            "STFC_STRUCTURE",
            &inputs,
            &["ECHOSTRUCT".to_string()],
            CompatOptions::default(),
        )
        .expect("call with inactive ECHOSTRUCT failed");
    assert!(
        !trimmed.contains_key("ECHOSTRUCT"),
        "inactive output must be omitted, got keys: {:?}",
        trimmed.keys().collect::<Vec<_>>()
    );
    assert!(trimmed.contains_key("RESPTEXT"));
}

#[test]
#[ignore]
fn inactive_tables_param_is_omitted() {
    // The guide's canonical use case (§4.2.2): don't request a big table.
    // RFC_READ_TABLE on T000 with the DATA table deactivated.
    let conn = connect();
    let mut inputs = IndexMap::new();
    inputs.insert("QUERY_TABLE".into(), RfcValue::String("T000".into()));

    let result = conn
        .call(
            "RFC_READ_TABLE",
            &inputs,
            &["DATA".to_string()],
            CompatOptions::default(),
        )
        .expect("RFC_READ_TABLE with inactive DATA failed");
    assert!(
        !result.contains_key("DATA"),
        "inactive TABLES param must be omitted, got keys: {:?}",
        result.keys().collect::<Vec<_>>()
    );
    // The field list is still delivered.
    assert!(result.contains_key("FIELDS"));
}

#[test]
#[ignore]
fn inactive_input_is_not_sent() {
    // An inactive INPUT parameter is not sent even when a value was set
    // for it first (guide §4.2.2's "no way back" scenario — deactivation
    // is the documented undo for a setter). Probe fact behind the pipeline
    // order: set-then-deactivate wins for deactivation, while
    // deactivate-then-set RE-ACTIVATES the parameter and sends the value.
    let conn = connect();
    let mut inputs = IndexMap::new();
    inputs.insert("REQUTEXT".into(), RfcValue::String("ignored".into()));

    let result = conn
        .call(
            "STFC_CONNECTION",
            &inputs,
            &["REQUTEXT".to_string()],
            CompatOptions::default(),
        )
        .expect("call with inactive input failed");
    println!("\n=== inactive input probe (value set, then param deactivated) ===");
    println!("ECHOTEXT: {:?}", result.get("ECHOTEXT"));
    match result.get("ECHOTEXT") {
        Some(RfcValue::Char(s)) => assert_eq!(
            s.trim_end_matches(' '),
            "",
            "REQUTEXT was sent despite being deactivated"
        ),
        other => panic!("ECHOTEXT should be Char, got: {other:?}"),
    }
}

#[test]
#[ignore]
fn unknown_inactive_name_errors() {
    let conn = connect();
    let inputs = IndexMap::new();
    let err = conn
        .call(
            "STFC_CONNECTION",
            &inputs,
            &["NO_SUCH_PARAM".to_string()],
            CompatOptions::default(),
        )
        .expect_err("unknown parameter name must error");
    println!("\n=== unknown inactive name probe ===");
    println!("error: {err:?}");
    if let Some(info) = err.info() {
        println!(
            "code={} group={} key=[{}] message=[{}]",
            info.code, info.group, info.key, info.message
        );
    }
}

/// Probe: what does the SDK hand back if you DO read an inactive output
/// after invoke? (Our read_outputs skips them — this records what the skip
/// is protecting callers from.) Raw FFI, setter_probe style.
#[test]
#[ignore]
fn probe_reading_inactive_output_raw() {
    let conn = connect();
    let sdk = pyrfcx_core::ffi::sdk().expect("sdk");
    let mut error_info = pyrfcx_core::ffi::zero_error_info();

    let fm_name = pyrfcx_core::uc::encode_unchecked("STFC_STRUCTURE");
    // The metadata-cache lock: tests in this binary run in parallel.
    let cache = pyrfcx_core::ffi::lock_metadata_cache();
    // Safety: conn.handle() is a valid connection, fm_name is null-terminated.
    let func_desc =
        unsafe { sdk.RfcGetFunctionDesc(conn.handle(), fm_name.as_ptr(), &mut error_info) };
    drop(cache);
    assert!(!func_desc.is_null(), "RfcGetFunctionDesc failed");

    // Safety: func_desc is valid.
    let func = unsafe { sdk.RfcCreateFunction(func_desc, &mut error_info) };
    assert!(!func.is_null(), "RfcCreateFunction failed");

    let echostruct = pyrfcx_core::uc::encode_unchecked("ECHOSTRUCT");
    // Safety: func is valid, echostruct null-terminated.
    let rc = unsafe { sdk.RfcSetParameterActive(func, echostruct.as_ptr(), 0, &mut error_info) };
    assert_eq!(
        rc,
        pyrfcx_core::ffi::bindings::RFC_RC::RFC_OK,
        "SetParameterActive"
    );

    // Safety: both handles valid, inputs are all defaulted.
    let rc = unsafe { sdk.RfcInvoke(conn.handle(), func, &mut error_info) };
    assert_eq!(rc, pyrfcx_core::ffi::bindings::RFC_RC::RFC_OK, "RfcInvoke");

    println!("\n=== raw read of an INACTIVE output (STFC_STRUCTURE.ECHOSTRUCT) ===");
    let mut is_active: std::ffi::c_int = -1;
    // Safety: func valid, out-params valid.
    let rc = unsafe {
        sdk.RfcIsParameterActive(func, echostruct.as_ptr(), &mut is_active, &mut error_info)
    };
    println!("RfcIsParameterActive: rc={rc:?}, is_active={is_active}");

    let mut sh: pyrfcx_core::ffi::bindings::RFC_STRUCTURE_HANDLE = std::ptr::null_mut();
    // Safety: func valid, out-params valid.
    let rc = unsafe { sdk.RfcGetStructure(func, echostruct.as_ptr(), &mut sh, &mut error_info) };
    println!("RfcGetStructure on inactive param: rc={rc:?}");
    if rc == pyrfcx_core::ffi::bindings::RFC_RC::RFC_OK {
        let field = pyrfcx_core::uc::encode_unchecked("RFCDATA1");
        let mut buf = vec![0u16; 64];
        let mut len: u32 = 0;
        // Safety: sh valid, buffer sized, out-params valid.
        let rc = unsafe {
            sdk.RfcGetString(
                sh,
                field.as_ptr(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut len,
                &mut error_info,
            )
        };
        println!(
            "RfcGetString(RFCDATA1): rc={rc:?}, value=[{}]",
            String::from_utf16_lossy(&buf[..len as usize])
        );
    }

    // Safety: func valid.
    unsafe { sdk.RfcDestroyFunction(func, &mut error_info) };
}
