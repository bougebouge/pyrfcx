// Setter-parity probe.
//
// Compares RfcSetChars vs RfcSetString on over-long input for CHAR fields,
// and records surrogate-straddle behavior. Container-local: no RfcInvoke.
//
// Run with: cargo test -p pyrfcx-core --test setter_probe -- --ignored

use std::sync::Once;

use pyrfcx_core::ffi;

static INI_INIT: Once = Once::new();

fn ensure_ini_path() {
    INI_INIT.call_once(|| {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        ffi::set_ini_path(workspace_root.to_str().unwrap()).expect("RfcSetIniPath failed");
    });
}

/// Connect, get the STFC_STRUCTURE descriptor, create a function handle,
/// and get the IMPORTSTRUCT structure handle for direct field manipulation.
fn setup() -> (
    &'static ffi::bindings::SapNwRfc,
    ffi::bindings::RFC_FUNCTION_DESC_HANDLE,
) {
    ensure_ini_path();
    let sdk = ffi::sdk().expect("SDK not loaded");

    // Open a connection just to populate the repository cache.
    let params = pyrfcx_core::ConnectionParams::new().set("dest", "TEST_NW750");
    let conn = pyrfcx_core::Connection::open(&params).expect("connect failed");

    let fm_name = pyrfcx_core::uc::encode_unchecked("STFC_STRUCTURE");
    let mut error_info = ffi::zero_error_info();
    // The metadata-cache lock: tests in this binary run in parallel.
    let cache = pyrfcx_core::ffi::lock_metadata_cache();
    // Safety: conn.handle() is a valid connection, fm_name is null-terminated.
    let func_desc =
        unsafe { sdk.RfcGetFunctionDesc(conn.handle(), fm_name.as_ptr(), &mut error_info) };
    drop(cache);
    assert!(!func_desc.is_null(), "RfcGetFunctionDesc failed");

    // Don't need the connection anymore — descriptor is cached.
    drop(conn);

    (sdk, func_desc)
}

/// Create a fresh function handle and get the IMPORTSTRUCT structure handle.
fn make_container(
    sdk: &ffi::bindings::SapNwRfc,
    func_desc: ffi::bindings::RFC_FUNCTION_DESC_HANDLE,
) -> (
    ffi::bindings::RFC_FUNCTION_HANDLE,
    ffi::bindings::RFC_STRUCTURE_HANDLE,
) {
    let mut error_info = ffi::zero_error_info();
    // Safety: func_desc is valid and cached.
    let func = unsafe { sdk.RfcCreateFunction(func_desc, &mut error_info) };
    assert!(!func.is_null(), "RfcCreateFunction failed");

    let param_name = pyrfcx_core::uc::encode_unchecked("IMPORTSTRUCT");
    error_info = ffi::zero_error_info();
    let mut struct_handle: ffi::bindings::RFC_STRUCTURE_HANDLE = std::ptr::null_mut();
    // Safety: func is valid, param_name is null-terminated.
    let rc = unsafe {
        sdk.RfcGetStructure(
            func,
            param_name.as_ptr(),
            &mut struct_handle,
            &mut error_info,
        )
    };
    assert_eq!(rc, ffi::bindings::RFC_RC::RFC_OK, "RfcGetStructure failed");
    assert!(!struct_handle.is_null());

    (func, struct_handle)
}

/// Read back a CHAR field's raw UTF-16 content.
fn read_chars(
    sdk: &ffi::bindings::SapNwRfc,
    handle: ffi::bindings::RFC_STRUCTURE_HANDLE,
    field: &str,
    char_count: usize,
) -> Vec<u16> {
    let name = pyrfcx_core::uc::encode_unchecked(field);
    let mut buf = vec![0u16; char_count];
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name is null-terminated, buf is large enough.
    let rc = unsafe {
        sdk.RfcGetChars(
            handle,
            name.as_ptr(),
            buf.as_mut_ptr(),
            char_count as std::ffi::c_uint,
            &mut error_info,
        )
    };
    assert_eq!(rc, ffi::bindings::RFC_RC::RFC_OK, "RfcGetChars failed");
    buf
}

/// Destroy a function handle.
fn destroy(sdk: &ffi::bindings::SapNwRfc, func: ffi::bindings::RFC_FUNCTION_HANDLE) {
    let mut error_info = ffi::zero_error_info();
    // Safety: func is a valid function handle from RfcCreateFunction.
    unsafe { sdk.RfcDestroyFunction(func, &mut error_info) };
}

/// Format UTF-16 content as a Rust string for display.
fn u16_to_string(buf: &[u16]) -> String {
    String::from_utf16_lossy(buf)
}

// ── Probe 1: Over-long value via RfcSetChars vs RfcSetString ──────────

#[test]
#[ignore]
fn probe_overlong_setchars_vs_setstring() {
    let (sdk, func_desc) = setup();

    // RFCCHAR4 has width 4 (ucLength = 8 bytes). Write "ABCDEF" (6 chars).
    let field = "RFCCHAR4";
    let field_name = pyrfcx_core::uc::encode_unchecked(field);
    let overlong = "ABCDEF";
    let overlong_uc = pyrfcx_core::uc::encode_unchecked(overlong);
    // encode_unchecked() adds a null terminator; char count excludes it.
    let overlong_len = (overlong_uc.len() - 1) as std::ffi::c_uint;

    // ── RfcSetChars ───────────────────────────────────────────────────
    let (func1, struct1) = make_container(sdk, func_desc);
    let mut error_info = ffi::zero_error_info();
    // Safety: struct1 is valid, field_name and overlong_uc are null-terminated.
    let rc_chars = unsafe {
        sdk.RfcSetChars(
            struct1,
            field_name.as_ptr(),
            overlong_uc.as_ptr(),
            overlong_len,
            &mut error_info,
        )
    };
    let err_chars = ffi::extract_error_info("RfcSetChars", &error_info);
    let content_chars = if rc_chars == ffi::bindings::RFC_RC::RFC_OK {
        Some(read_chars(sdk, struct1, field, 4))
    } else {
        None
    };
    destroy(sdk, func1);

    // ── RfcSetString ──────────────────────────────────────────────────
    let (func2, struct2) = make_container(sdk, func_desc);
    error_info = ffi::zero_error_info();
    // Safety: struct2 is valid, field_name and overlong_uc are null-terminated.
    let rc_string = unsafe {
        sdk.RfcSetString(
            struct2,
            field_name.as_ptr(),
            overlong_uc.as_ptr(),
            overlong_len,
            &mut error_info,
        )
    };
    let err_string = ffi::extract_error_info("RfcSetString", &error_info);
    let content_string = if rc_string == ffi::bindings::RFC_RC::RFC_OK {
        Some(read_chars(sdk, struct2, field, 4))
    } else {
        None
    };
    destroy(sdk, func2);

    // ── Report ────────────────────────────────────────────────────────
    println!("\n=== Probe 1: Over-long CHAR(4) with \"ABCDEF\" (6 units) ===");
    println!(
        "RfcSetChars: rc={:?}, code={}, group={}, msg={:?}",
        rc_chars, err_chars.code, err_chars.group, err_chars.message
    );
    if let Some(ref buf) = content_chars {
        println!("  read-back (UTF-16): {:?}", buf);
        println!("  read-back (string): {:?}", u16_to_string(buf));
    }
    println!(
        "RfcSetString: rc={:?}, code={}, group={}, msg={:?}",
        rc_string, err_string.code, err_string.group, err_string.message
    );
    if let Some(ref buf) = content_string {
        println!("  read-back (UTF-16): {:?}", buf);
        println!("  read-back (string): {:?}", u16_to_string(buf));
    }

    // Assert parity: both should succeed with identical truncated content.
    assert_eq!(rc_chars, rc_string, "return codes differ");
    assert_eq!(content_chars, content_string, "read-back content differs");
}

// ── Probe 2: Surrogate pair straddling the width ──────────────────────

#[test]
#[ignore]
fn probe_surrogate_straddle() {
    let (sdk, func_desc) = setup();

    // "abc𝄞" = 3 BMP + 1 supplementary (2 UTF-16 units) = 5 units into CHAR(4).
    let field = "RFCCHAR4";
    let field_name = pyrfcx_core::uc::encode_unchecked(field);
    let input = "abc𝄞";
    let input_uc = pyrfcx_core::uc::encode_unchecked(input);
    let input_len = (input_uc.len() - 1) as std::ffi::c_uint;

    // ── RfcSetChars ───────────────────────────────────────────────────
    let (func1, struct1) = make_container(sdk, func_desc);
    let mut error_info = ffi::zero_error_info();
    // Safety: struct1 is valid, field_name and input_uc are null-terminated.
    let rc_chars = unsafe {
        sdk.RfcSetChars(
            struct1,
            field_name.as_ptr(),
            input_uc.as_ptr(),
            input_len,
            &mut error_info,
        )
    };
    let err_chars = ffi::extract_error_info("RfcSetChars", &error_info);
    let content_chars = if rc_chars == ffi::bindings::RFC_RC::RFC_OK {
        Some(read_chars(sdk, struct1, field, 4))
    } else {
        None
    };
    destroy(sdk, func1);

    // ── RfcSetString ──────────────────────────────────────────────────
    let (func2, struct2) = make_container(sdk, func_desc);
    error_info = ffi::zero_error_info();
    // Safety: struct2 is valid, field_name and input_uc are null-terminated.
    let rc_string = unsafe {
        sdk.RfcSetString(
            struct2,
            field_name.as_ptr(),
            input_uc.as_ptr(),
            input_len,
            &mut error_info,
        )
    };
    let err_string = ffi::extract_error_info("RfcSetString", &error_info);
    let content_string = if rc_string == ffi::bindings::RFC_RC::RFC_OK {
        Some(read_chars(sdk, struct2, field, 4))
    } else {
        None
    };
    destroy(sdk, func2);

    // ── Report ────────────────────────────────────────────────────────
    println!("\n=== Probe 2: Surrogate straddle CHAR(4) with \"abc𝄞\" (5 units) ===");
    println!(
        "RfcSetChars: rc={:?}, code={}, group={}, msg={:?}",
        rc_chars, err_chars.code, err_chars.group, err_chars.message
    );
    if let Some(ref buf) = content_chars {
        println!("  read-back (UTF-16): {:?}", buf);
        println!("  read-back (string): {:?}", u16_to_string(buf));
    }
    println!(
        "RfcSetString: rc={:?}, code={}, group={}, msg={:?}",
        rc_string, err_string.code, err_string.group, err_string.message
    );
    if let Some(ref buf) = content_string {
        println!("  read-back (UTF-16): {:?}", buf);
        println!("  read-back (string): {:?}", u16_to_string(buf));
    }

    assert_eq!(
        rc_chars, rc_string,
        "return codes differ on surrogate straddle"
    );
    assert_eq!(
        content_chars, content_string,
        "read-back content differs on surrogate straddle"
    );
}

// ── Probe 3: Exact-width value — sanity check ────────────────────────

#[test]
#[ignore]
fn probe_exact_width() {
    let (sdk, func_desc) = setup();

    let field = "RFCCHAR4";
    let field_name = pyrfcx_core::uc::encode_unchecked(field);
    let exact = "ABCD";
    let exact_uc = pyrfcx_core::uc::encode_unchecked(exact);
    let exact_len = (exact_uc.len() - 1) as std::ffi::c_uint;

    // ── RfcSetChars ───────────────────────────────────────────────────
    let (func1, struct1) = make_container(sdk, func_desc);
    let mut error_info = ffi::zero_error_info();
    // Safety: struct1 is valid, all pointers valid and null-terminated.
    let rc_chars = unsafe {
        sdk.RfcSetChars(
            struct1,
            field_name.as_ptr(),
            exact_uc.as_ptr(),
            exact_len,
            &mut error_info,
        )
    };
    assert_eq!(
        rc_chars,
        ffi::bindings::RFC_RC::RFC_OK,
        "RfcSetChars exact-width failed"
    );
    let content_chars = read_chars(sdk, struct1, field, 4);
    destroy(sdk, func1);

    // ── RfcSetString ──────────────────────────────────────────────────
    let (func2, struct2) = make_container(sdk, func_desc);
    error_info = ffi::zero_error_info();
    // Safety: struct2 is valid, all pointers valid and null-terminated.
    let rc_string = unsafe {
        sdk.RfcSetString(
            struct2,
            field_name.as_ptr(),
            exact_uc.as_ptr(),
            exact_len,
            &mut error_info,
        )
    };
    assert_eq!(
        rc_string,
        ffi::bindings::RFC_RC::RFC_OK,
        "RfcSetString exact-width failed"
    );
    let content_string = read_chars(sdk, struct2, field, 4);
    destroy(sdk, func2);

    // ── Report & assert ───────────────────────────────────────────────
    println!("\n=== Probe 3: Exact-width CHAR(4) with \"ABCD\" ===");
    println!(
        "RfcSetChars  read-back: {:?} = {:?}",
        content_chars,
        u16_to_string(&content_chars)
    );
    println!(
        "RfcSetString read-back: {:?} = {:?}",
        content_string,
        u16_to_string(&content_string)
    );

    assert_eq!(content_chars, content_string, "exact-width content differs");
    let expected: Vec<u16> = "ABCD".encode_utf16().collect();
    assert_eq!(content_chars, expected, "content not ABCD");
}

// ── Probe 4: NUMC which-end ──────────────────────────────────────────
// STFC_STRUCTURE has no NUMC field. The NUMC probe is in live_call.rs
// (probe 4: NUMC over-long input).
