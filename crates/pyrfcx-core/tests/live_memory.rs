// Live memory measurement: where process memory goes when one large RFC
// table is read. Measurement only, no assertions on the figures.
//
// One measurement per process: run ONE test per `cargo test` invocation, in
// release, e.g.
//   cargo test --release -p pyrfcx-core --test live_memory d1_raw_invoke -- --ignored --nocapture
//
// Each test prints one line that starts with `MEMJSON ` followed by a JSON
// object. All memory figures are bytes.
//
// Parameters (environment variables):
//   MEM_DEST    destination in sapnwrfc.ini        (default TEST_NW750)
//   MEM_FUNC    function module                    (default RFC_READ_TABLE)
//   MEM_INPUTS  scalar inputs, `NAME=VALUE;NAME=VALUE`; `{rows}` is replaced
//               by MEM_ROWS     (default QUERY_TABLE=DD03L;ROWCOUNT={rows})
//   MEM_TABLE   table parameter to measure         (default DATA)
//   MEM_ROWS    row count                          (default 10000)
//   MEM_INI_DIR directory holding sapnwrfc.ini     (default workspace root)
//   MEM_SAMPLER_MS  D2 sampler interval, 0 = off   (default 2)
//   MEM_STATS   0 = skip the d1 content statistics (default on)
//   MEM_CHUNK   D3 rows per chunk                  (default 4096)
//
// MEM_INPUTS keys of the form `STRUCT.FIELD` set one field of a structure
// parameter, e.g. `IS_INTERVAL.DATE_FROM=20260915`.
//
// Check the ini for RFC_TRACE / CPIC_TRACE_LEVEL / RFC_SOCKET_TRACE first:
// any trace level distorts the figures. Point MEM_INI_DIR at a copy with
// tracing off.
//
// All tests are #[ignore]: they need the live destination.
//
// Windows only: the figures are Windows process memory counters
// (GetProcessMemoryInfo), which have no one-to-one equivalent elsewhere.
#![cfg(windows)]

use std::sync::Arc;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use indexmap::IndexMap;

use pyrfcx_core::ffi::bindings::{
    RFC_FIELD_DESC, RFC_FUNCTION_DESC_HANDLE, RFC_FUNCTION_HANDLE, RFC_PARAMETER_DESC, RFC_RC,
    RFC_STRUCTURE_HANDLE, RFC_TABLE_HANDLE, RFC_TYPE_DESC_HANDLE, RFCTYPE,
};
use pyrfcx_core::{CompatOptions, Connection, ConnectionParams, RfcValue};

// ── process memory (GetProcessMemoryInfo) ─────────────────────────────

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ProcessMemoryCountersEx {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_usage: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    // kernel32 exports the psapi function under the K32 prefix (Windows 7+).
    fn K32GetProcessMemoryInfo(
        process: *mut std::ffi::c_void,
        counters: *mut ProcessMemoryCountersEx,
        cb: u32,
    ) -> i32;
}

#[derive(Clone, Copy)]
struct Mem {
    ws: usize,
    peak_ws: usize,
    commit: usize,      // PrivateUsage
    peak_commit: usize, // PeakPagefileUsage
}

fn mem() -> Mem {
    let mut c = ProcessMemoryCountersEx {
        cb: std::mem::size_of::<ProcessMemoryCountersEx>() as u32,
        ..Default::default()
    };
    // Safety: GetCurrentProcess returns a pseudo-handle that needs no
    // closing; `c` is a correctly sized, writable PROCESS_MEMORY_COUNTERS_EX
    // and `cb` states its size.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    assert!(ok != 0, "GetProcessMemoryInfo failed");
    Mem {
        ws: c.working_set_size,
        peak_ws: c.peak_working_set_size,
        commit: c.private_usage,
        peak_commit: c.peak_pagefile_usage,
    }
}

fn mem_json(m: &Mem, zero: &Mem) -> String {
    format!(
        "{{\"commit\":{},\"ws\":{},\"peak_commit\":{},\"peak_ws\":{}}}",
        m.commit as i64 - zero.commit as i64,
        m.ws as i64 - zero.ws as i64,
        m.peak_commit as i64 - zero.commit as i64,
        m.peak_ws as i64 - zero.ws as i64,
    )
}

/// Background thread that polls current commit and working set and keeps
/// the maximum of each. Used where the code under test is opaque.
struct Sampler {
    stop: Arc<AtomicBool>,
    max_commit: Arc<AtomicUsize>,
    max_ws: Arc<AtomicUsize>,
    samples: Arc<AtomicUsize>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Sampler {
    fn start(interval: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let max_commit = Arc::new(AtomicUsize::new(0));
        let max_ws = Arc::new(AtomicUsize::new(0));
        let samples = Arc::new(AtomicUsize::new(0));
        let (s, mc, mw, n) = (
            stop.clone(),
            max_commit.clone(),
            max_ws.clone(),
            samples.clone(),
        );
        let join = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                let m = mem();
                mc.fetch_max(m.commit, Ordering::Relaxed);
                mw.fetch_max(m.ws, Ordering::Relaxed);
                n.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(interval);
            }
        });
        Sampler {
            stop,
            max_commit,
            max_ws,
            samples,
            join: Some(join),
        }
    }

    /// Stop and return (max commit, max working set, sample count).
    fn finish(mut self) -> (usize, usize, usize) {
        self.stop.store(true, Ordering::Relaxed);
        self.join.take().unwrap().join().unwrap();
        (
            self.max_commit.load(Ordering::Relaxed),
            self.max_ws.load(Ordering::Relaxed),
            self.samples.load(Ordering::Relaxed),
        )
    }
}

// ── parameters and connection ─────────────────────────────────────────

struct Params {
    dest: String,
    func: String,
    inputs: Vec<(String, String)>,
    table: String,
    rows: u64,
}

fn params() -> Params {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let rows: u64 = env("MEM_ROWS", "10000").parse().expect("MEM_ROWS");
    let raw_inputs = env("MEM_INPUTS", "QUERY_TABLE=DD03L;ROWCOUNT={rows}");
    let inputs = raw_inputs
        .split(';')
        .filter(|s| !s.trim().is_empty())
        .map(|kv| {
            let (k, v) = kv
                .split_once('=')
                .expect("MEM_INPUTS entry needs NAME=VALUE");
            (
                k.trim().to_string(),
                v.trim().replace("{rows}", &rows.to_string()),
            )
        })
        .collect();
    Params {
        dest: env("MEM_DEST", "TEST_NW750"),
        func: env("MEM_FUNC", "RFC_READ_TABLE"),
        inputs,
        table: env("MEM_TABLE", "DATA"),
        rows,
    }
}

static INI_INIT: Once = Once::new();

fn connect(p: &Params) -> Connection {
    INI_INIT.call_once(|| {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        // MEM_INI_DIR: directory of an alternative sapnwrfc.ini. An RFC trace
        // level in the ini distorts both time and memory; measure with a
        // copy that has tracing off.
        let dir = std::env::var("MEM_INI_DIR")
            .unwrap_or_else(|_| workspace_root.to_str().unwrap().to_string());
        pyrfcx_core::ffi::set_ini_path(&dir).expect("RfcSetIniPath failed");
    });
    Connection::open(&ConnectionParams::new().set("dest", &p.dest)).expect("connect failed")
}

fn sdk() -> &'static pyrfcx_core::ffi::bindings::SapNwRfc {
    pyrfcx_core::ffi::sdk().expect("sdk")
}

fn sdk_version() -> String {
    pyrfcx_core::ffi::get_version()
        .map(|v| v.3)
        .unwrap_or_default()
}

fn fail(what: &'static str, ei: &pyrfcx_core::ffi::bindings::RFC_ERROR_INFO) -> ! {
    let info = pyrfcx_core::ffi::extract_error_info(what, ei);
    panic!(
        "{what} failed: code={} group={} key=[{}] message=[{}]",
        info.code, info.group, info.key, info.message
    );
}

fn func_desc(conn: &Connection, name: &str) -> RFC_FUNCTION_DESC_HANDLE {
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    let name_uc = pyrfcx_core::uc::encode_unchecked(name);
    // The metadata-cache lock: tests in this binary run in parallel.
    let cache = pyrfcx_core::ffi::lock_metadata_cache();
    // Safety: conn.handle() is an open connection; name_uc is
    // null-terminated. The descriptor is SDK-cached; we do not free it.
    let d = unsafe { sdk().RfcGetFunctionDesc(conn.handle(), name_uc.as_ptr(), &mut ei) };
    drop(cache);
    if d.is_null() {
        fail("RfcGetFunctionDesc", &ei);
    }
    d
}

fn param_desc(fd: RFC_FUNCTION_DESC_HANDLE, name: &str) -> RFC_PARAMETER_DESC {
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    let name_uc = pyrfcx_core::uc::encode_unchecked(name);
    // Safety: RFC_PARAMETER_DESC is a plain C struct; all-zero is valid as
    // an out-buffer.
    let mut pd: RFC_PARAMETER_DESC = unsafe { std::mem::zeroed() };
    // Safety: fd is a valid descriptor, name_uc null-terminated, pd and ei
    // are valid out-params.
    let rc = unsafe { sdk().RfcGetParameterDescByName(fd, name_uc.as_ptr(), &mut pd, &mut ei) };
    if rc != RFC_RC::RFC_OK {
        fail("RfcGetParameterDescByName", &ei);
    }
    pd
}

/// Type length of the table parameter's row type: (nucLength, ucLength).
fn row_type_length(fd: RFC_FUNCTION_DESC_HANDLE, table: &str) -> (u32, u32) {
    let pd = param_desc(fd, table);
    assert!(
        pd.type_ == RFCTYPE::RFCTYPE_TABLE,
        "{table} is not a table parameter (direction {:?})",
        pd.direction
    );
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    let (mut nuc, mut uc) = (0u32, 0u32);
    // Safety: typeDescHandle comes from a valid parameter descriptor; out
    // params are valid.
    let rc = unsafe { sdk().RfcGetTypeLength(pd.typeDescHandle, &mut nuc, &mut uc, &mut ei) };
    if rc != RFC_RC::RFC_OK {
        fail("RfcGetTypeLength", &ei);
    }
    (nuc, uc)
}

fn create_function(fd: RFC_FUNCTION_DESC_HANDLE) -> RFC_FUNCTION_HANDLE {
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    // Safety: fd is a valid descriptor.
    let f = unsafe { sdk().RfcCreateFunction(fd, &mut ei) };
    if f.is_null() {
        fail("RfcCreateFunction", &ei);
    }
    f
}

/// Sets the inputs on the raw function handle. A key `STRUCT.FIELD` writes
/// one field of a structure parameter (RfcGetStructure, then RfcSetString
/// on the structure handle); a plain key writes a scalar parameter.
fn set_inputs_raw(f: RFC_FUNCTION_HANDLE, inputs: &[(String, String)]) {
    for (k, v) in inputs {
        let mut ei = pyrfcx_core::ffi::zero_error_info();
        let (target, field) = match k.split_once('.') {
            Some((s, fld)) => {
                let s_uc = pyrfcx_core::uc::encode_unchecked(s);
                let mut sh: RFC_STRUCTURE_HANDLE = std::ptr::null_mut();
                // Safety: f is valid, s_uc null-terminated, sh a valid out-param.
                let rc = unsafe { sdk().RfcGetStructure(f, s_uc.as_ptr(), &mut sh, &mut ei) };
                if rc != RFC_RC::RFC_OK {
                    fail("RfcGetStructure", &ei);
                }
                (sh, fld.to_string())
            }
            None => (f, k.clone()),
        };
        let k_uc = pyrfcx_core::uc::encode_unchecked(&field);
        let v_uc = pyrfcx_core::uc::encode_unchecked(v);
        // Safety: target is a valid function or structure handle; both
        // strings are null-terminated; the length excludes the terminator.
        let rc = unsafe {
            sdk().RfcSetString(
                target,
                k_uc.as_ptr(),
                v_uc.as_ptr(),
                (v_uc.len() - 1) as u32,
                &mut ei,
            )
        };
        if rc != RFC_RC::RFC_OK {
            fail("RfcSetString", &ei);
        }
    }
}

fn invoke_raw(conn: &Connection, f: RFC_FUNCTION_HANDLE) -> Duration {
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    let t = Instant::now();
    // Safety: connection and function handle are valid; inputs are set.
    let rc = unsafe { sdk().RfcInvoke(conn.handle(), f, &mut ei) };
    let d = t.elapsed();
    if rc != RFC_RC::RFC_OK {
        fail("RfcInvoke", &ei);
    }
    d
}

fn table_handle(f: RFC_FUNCTION_HANDLE, table: &str) -> RFC_TABLE_HANDLE {
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    let name_uc = pyrfcx_core::uc::encode_unchecked(table);
    let mut th: RFC_TABLE_HANDLE = std::ptr::null_mut();
    // Safety: f is valid, name_uc null-terminated, th is a valid out-param.
    let rc = unsafe { sdk().RfcGetTable(f, name_uc.as_ptr(), &mut th, &mut ei) };
    if rc != RFC_RC::RFC_OK {
        fail("RfcGetTable", &ei);
    }
    th
}

fn row_count(th: RFC_TABLE_HANDLE) -> u32 {
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    let mut n = 0u32;
    // Safety: th is a valid table handle; n is a valid out-param.
    let rc = unsafe { sdk().RfcGetRowCount(th, &mut n, &mut ei) };
    if rc != RFC_RC::RFC_OK {
        fail("RfcGetRowCount", &ei);
    }
    n
}

fn destroy_function(f: RFC_FUNCTION_HANDLE) {
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    // Safety: f is a valid function handle, destroyed exactly once here.
    unsafe { sdk().RfcDestroyFunction(f, &mut ei) };
}

fn header(test: &str, p: &Params) -> String {
    let inputs: Vec<String> = p.inputs.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!(
        "\"test\":\"{test}\",\"func\":\"{}\",\"table\":\"{}\",\"rows_requested\":{},\"inputs\":\"{}\",\"sdk\":\"{}\"",
        p.func,
        p.table,
        p.rows,
        inputs.join(";"),
        sdk_version()
    )
}

// ── row type layout and content statistics ────────────────────────────

fn abap_name(n: &[u16]) -> String {
    let end = n.iter().position(|&c| c == 0).unwrap_or(n.len());
    String::from_utf16_lossy(&n[..end])
}

fn type_length(t: RFC_TYPE_DESC_HANDLE) -> (u32, u32) {
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    let (mut nuc, mut uc) = (0u32, 0u32);
    // Safety: t is a valid type descriptor; out params are valid.
    let rc = unsafe { sdk().RfcGetTypeLength(t, &mut nuc, &mut uc, &mut ei) };
    if rc != RFC_RC::RFC_OK {
        fail("RfcGetTypeLength", &ei);
    }
    (nuc, uc)
}

fn type_fields(t: RFC_TYPE_DESC_HANDLE) -> Vec<RFC_FIELD_DESC> {
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    let mut n = 0u32;
    // Safety: t is a valid type descriptor; n is a valid out-param.
    let rc = unsafe { sdk().RfcGetFieldCount(t, &mut n, &mut ei) };
    if rc != RFC_RC::RFC_OK {
        fail("RfcGetFieldCount", &ei);
    }
    (0..n)
        .map(|i| {
            // Safety: RFC_FIELD_DESC is a plain C struct; zeroed is a valid out-buffer.
            let mut fd: RFC_FIELD_DESC = unsafe { std::mem::zeroed() };
            // Safety: t valid, i < field count, fd and ei valid out-params.
            let rc = unsafe { sdk().RfcGetFieldDescByIndex(t, i, &mut fd, &mut ei) };
            if rc != RFC_RC::RFC_OK {
                fail("RfcGetFieldDescByIndex", &ei);
            }
            fd
        })
        .collect()
}

/// JSON list of the fields of a type: name, type code, lengths, and for a
/// nested structure or table the type length of its line type.
fn layout_json(t: RFC_TYPE_DESC_HANDLE) -> String {
    let parts: Vec<String> = type_fields(t)
        .iter()
        .map(|f| {
            let nested = if (f.type_ == RFCTYPE::RFCTYPE_TABLE
                || f.type_ == RFCTYPE::RFCTYPE_STRUCTURE)
                && !f.typeDescHandle.is_null()
            {
                let (nn, nu) = type_length(f.typeDescHandle);
                format!(
                    ",\"line_nuc\":{nn},\"line_uc\":{nu},\"line\":{}",
                    layout_json(f.typeDescHandle)
                )
            } else {
                String::new()
            };
            format!(
                "{{\"name\":\"{}\",\"type\":{},\"nuc\":{},\"uc\":{},\"uc_off\":{}{nested}}}",
                abap_name(&f.name),
                f.type_.0,
                f.nucLength,
                f.ucLength,
                f.ucOffset
            )
        })
        .collect();
    format!("[{}]", parts.join(","))
}

/// Content statistics of one container type, accumulated over rows.
#[derive(Default)]
struct Content {
    /// Sum of STRING lengths per field name (UTF-16 units).
    string_len: IndexMap<String, u64>,
    /// Sum of nested-table row counts per field name.
    nested_rows: IndexMap<String, u64>,
    /// Fixed-width character fields: full width and rstripped length.
    fixed_chars: u64,
    trimmed_chars: u64,
    /// Characters in nested tables (all their fields, fixed full width).
    nested_chars: u64,
    nested_trimmed_chars: u64,
    nested_string_chars: u64,
}

fn is_fixed_char(t: RFCTYPE) -> bool {
    t == RFCTYPE::RFCTYPE_CHAR
        || t == RFCTYPE::RFCTYPE_NUM
        || t == RFCTYPE::RFCTYPE_DATE
        || t == RFCTYPE::RFCTYPE_TIME
}

/// Reads one row (structure handle) of the given type and adds its content.
/// `nested` = true accumulates into the nested-table counters.
fn row_content(
    row: RFC_STRUCTURE_HANDLE,
    fields: &[RFC_FIELD_DESC],
    nested: bool,
    c: &mut Content,
) {
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    let mut buf: Vec<u16> = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        let i = i as u32;
        if f.type_ == RFCTYPE::RFCTYPE_STRING {
            let mut len = 0u32;
            // Safety: row is a valid structure handle, i a STRING field index.
            let rc = unsafe { sdk().RfcGetStringLengthByIndex(row, i, &mut len, &mut ei) };
            if rc != RFC_RC::RFC_OK {
                fail("RfcGetStringLengthByIndex", &ei);
            }
            if nested {
                c.nested_string_chars += len as u64;
            } else {
                *c.string_len.entry(abap_name(&f.name)).or_default() += len as u64;
            }
        } else if f.type_ == RFCTYPE::RFCTYPE_TABLE && !nested {
            let mut th: RFC_TABLE_HANDLE = std::ptr::null_mut();
            // Safety: row valid, i a TABLE field index, th a valid out-param.
            let rc = unsafe { sdk().RfcGetTableByIndex(row, i, &mut th, &mut ei) };
            if rc != RFC_RC::RFC_OK {
                fail("RfcGetTableByIndex", &ei);
            }
            let n = row_count(th);
            *c.nested_rows.entry(abap_name(&f.name)).or_default() += n as u64;
            if n > 0 {
                let sub = type_fields(f.typeDescHandle);
                for j in 0..n {
                    // Safety: th valid, j < row count.
                    unsafe { sdk().RfcMoveTo(th, j, &mut ei) };
                    // Safety: th is positioned on row j.
                    let r = unsafe { sdk().RfcGetCurrentRow(th, &mut ei) };
                    if r.is_null() {
                        fail("RfcGetCurrentRow (nested)", &ei);
                    }
                    row_content(r, &sub, true, c);
                }
            }
        } else if is_fixed_char(f.type_) {
            let width = f.ucLength / 2;
            buf.resize(width as usize + 1, 0);
            let mut len = 0u32;
            // Safety: row valid, i a character-like field index; buf holds
            // width + 1 units, enough for the value plus terminator.
            let rc = unsafe {
                sdk().RfcGetStringByIndex(
                    row,
                    i,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut len,
                    &mut ei,
                )
            };
            if rc != RFC_RC::RFC_OK {
                fail("RfcGetStringByIndex", &ei);
            }
            let s = &buf[..len as usize];
            let trimmed = s
                .iter()
                .rposition(|&u| u != b' ' as u16)
                .map_or(0, |p| p + 1) as u64;
            if nested {
                c.nested_chars += width as u64;
                c.nested_trimmed_chars += trimmed;
            } else {
                c.fixed_chars += width as u64;
                c.trimmed_chars += trimmed;
            }
        }
    }
}

/// Content statistics of a whole table, as JSON. Runs after the memory
/// readings: it creates row handles and Rust buffers.
fn content_json(th: RFC_TABLE_HANDLE, line: RFC_TYPE_DESC_HANDLE) -> String {
    let fields = type_fields(line);
    let rows = row_count(th);
    let mut c = Content::default();
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    for i in 0..rows {
        // Safety: th valid, i < row count.
        unsafe { sdk().RfcMoveTo(th, i, &mut ei) };
        // Safety: th is positioned on row i.
        let r = unsafe { sdk().RfcGetCurrentRow(th, &mut ei) };
        if r.is_null() {
            fail("RfcGetCurrentRow", &ei);
        }
        row_content(r, &fields, false, &mut c);
    }
    let n = rows.max(1) as f64;
    let string_total: u64 = c.string_len.values().sum();
    let per_field: Vec<String> = c
        .string_len
        .iter()
        .map(|(k, v)| format!("\"{k}\":{:.2}", *v as f64 / n))
        .collect();
    let nested: Vec<String> = c
        .nested_rows
        .iter()
        .map(|(k, v)| format!("\"{k}\":{:.4}", *v as f64 / n))
        .collect();
    format!(
        "{{\"rows\":{rows},\"string_total_chars\":{string_total},\
         \"mean_string_len\":{{{}}},\"mean_nested_rows\":{{{}}},\
         \"mean_fixed_chars\":{:.2},\"mean_trimmed_fixed_chars\":{:.2},\
         \"mean_string_chars\":{:.2},\"mean_nested_fixed_chars\":{:.2},\
         \"mean_nested_trimmed_chars\":{:.2},\"mean_nested_string_chars\":{:.2},\
         \"mean_total_chars_full\":{:.2},\"mean_total_chars_trimmed\":{:.2}}}",
        per_field.join(","),
        nested.join(","),
        c.fixed_chars as f64 / n,
        c.trimmed_chars as f64 / n,
        string_total as f64 / n,
        c.nested_chars as f64 / n,
        c.nested_trimmed_chars as f64 / n,
        c.nested_string_chars as f64 / n,
        (c.fixed_chars + string_total + c.nested_chars + c.nested_string_chars) as f64 / n,
        (c.trimmed_chars + string_total + c.nested_trimmed_chars + c.nested_string_chars) as f64
            / n,
    )
}

/// Row count and line type length of every table parameter, as JSON. The
/// other output tables are in the container too.
fn all_tables_json(fd: RFC_FUNCTION_DESC_HANDLE, f: RFC_FUNCTION_HANDLE) -> String {
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    let mut n = 0u32;
    // Safety: fd valid; n a valid out-param.
    unsafe { sdk().RfcGetParameterCount(fd, &mut n, &mut ei) };
    let mut parts = Vec::new();
    for i in 0..n {
        // Safety: zeroed is a valid out-buffer for this plain C struct.
        let mut pd: RFC_PARAMETER_DESC = unsafe { std::mem::zeroed() };
        // Safety: fd valid, i < count.
        unsafe { sdk().RfcGetParameterDescByIndex(fd, i, &mut pd, &mut ei) };
        if pd.type_ == RFCTYPE::RFCTYPE_TABLE {
            let name = abap_name(&pd.name);
            let rows = row_count(table_handle(f, &name));
            let (_, uc) = type_length(pd.typeDescHandle);
            parts.push(format!("\"{name}\":{{\"rows\":{rows},\"line_uc\":{uc}}}"));
        }
    }
    format!("{{{}}}", parts.join(","))
}

/// Prints every parameter of MEM_FUNC with its type layout. No memory figures.
#[test]
#[ignore]
fn describe_shape() {
    let p = params();
    let conn = connect(&p);
    let fd = func_desc(&conn, &p.func);
    let mut ei = pyrfcx_core::ffi::zero_error_info();
    let mut n = 0u32;
    // Safety: fd valid; n a valid out-param.
    unsafe { sdk().RfcGetParameterCount(fd, &mut n, &mut ei) };
    for i in 0..n {
        // Safety: zeroed is a valid out-buffer for this plain C struct.
        let mut pd: RFC_PARAMETER_DESC = unsafe { std::mem::zeroed() };
        // Safety: fd valid, i < count.
        unsafe { sdk().RfcGetParameterDescByIndex(fd, i, &mut pd, &mut ei) };
        let layout = if pd.typeDescHandle.is_null() {
            String::new()
        } else {
            let (nn, nu) = type_length(pd.typeDescHandle);
            format!(
                ",\"line_nuc\":{nn},\"line_uc\":{nu},\"fields\":{}",
                layout_json(pd.typeDescHandle)
            )
        };
        println!(
            "PARAM {{\"name\":\"{}\",\"type\":{},\"direction\":{},\"optional\":{},\"nuc\":{},\"uc\":{}{layout}}}",
            abap_name(&pd.name),
            pd.type_.0,
            pd.direction.0,
            pd.optional,
            pd.nucLength,
            pd.ucLength
        );
    }
}

// ── D1 + F: raw FFI path, no value tree ───────────────────────────────

#[test]
#[ignore]
fn d1_raw_invoke() {
    let p = params();
    let conn = connect(&p);
    let fd = func_desc(&conn, &p.func);
    let (nuc, uc) = row_type_length(fd, &p.table);

    let zero = mem();
    let f = create_function(fd);
    set_inputs_raw(f, &p.inputs);
    let before_invoke = mem();
    let dur = invoke_raw(&conn, f);
    let after_invoke = mem();
    let th = table_handle(f, &p.table);
    let rows = row_count(th);
    // The statistics pass needs the container, so it runs after the
    // after_invoke reading; `after_stats` shows what the pass itself added.
    // MEM_STATS=0 skips the pass.
    let stats_on = std::env::var("MEM_STATS").map_or(true, |s| s != "0");
    let line = param_desc(fd, &p.table).typeDescHandle;
    let stats = if stats_on {
        content_json(th, line)
    } else {
        "null".to_string()
    };
    let tables = all_tables_json(fd, f);
    let after_stats = mem();
    destroy_function(f);
    let after_destroy = mem();

    println!(
        "MEMJSON {{{},\"rows\":{rows},\"type_nuc_len\":{nuc},\"type_uc_len\":{uc},\
         \"computed_container\":{},\"invoke_ms\":{},\"zero_abs\":{{\"commit\":{},\"ws\":{}}},\
         \"before_invoke\":{},\"after_invoke\":{},\"after_stats\":{},\"after_destroy\":{},\
         \"tables\":{tables},\"layout\":{},\"content\":{stats}}}",
        header("d1_raw_invoke", &p),
        rows as u64 * uc as u64,
        dur.as_millis(),
        zero.commit,
        zero.ws,
        mem_json(&before_invoke, &zero),
        mem_json(&after_invoke, &zero),
        mem_json(&after_stats, &zero),
        mem_json(&after_destroy, &zero),
        layout_json(line),
    );
}

// ── E: RfcDeleteCurrentRow from the last row backwards ─────────────────

#[test]
#[ignore]
fn e_delete_rows() {
    let p = params();
    let conn = connect(&p);
    let fd = func_desc(&conn, &p.func);
    let (_, uc) = row_type_length(fd, &p.table);

    let zero = mem();
    let f = create_function(fd);
    set_inputs_raw(f, &p.inputs);
    invoke_raw(&conn, f);
    let th = table_handle(f, &p.table);
    let rows = row_count(th);
    let after_invoke = mem();
    let tables = all_tables_json(fd, f);

    let mut marks = Vec::new();
    let quarter = |q: u32| (rows as u64 * q as u64 / 4) as u32;
    let mut next_q = 1;
    let t = Instant::now();
    for deleted in 1..=rows {
        let mut ei = pyrfcx_core::ffi::zero_error_info();
        // Safety: th is a valid, non-empty table handle.
        let rc = unsafe { sdk().RfcMoveToLastRow(th, &mut ei) };
        if rc != RFC_RC::RFC_OK {
            fail("RfcMoveToLastRow", &ei);
        }
        // Safety: th is positioned on its last row.
        let rc = unsafe { sdk().RfcDeleteCurrentRow(th, &mut ei) };
        if rc != RFC_RC::RFC_OK {
            fail("RfcDeleteCurrentRow", &ei);
        }
        while next_q <= 4 && deleted == quarter(next_q) {
            marks.push(format!(
                "\"deleted_{}pct\":{}",
                next_q * 25,
                mem_json(&mem(), &zero)
            ));
            next_q += 1;
        }
    }
    let delete_ms = t.elapsed().as_millis();
    let rows_left = row_count(th);
    destroy_function(f);
    let after_destroy = mem();

    println!(
        "MEMJSON {{{},\"rows\":{rows},\"type_uc_len\":{uc},\"computed_container\":{},\
         \"rows_left\":{rows_left},\"delete_ms\":{delete_ms},\"tables\":{tables},\
         \"after_invoke\":{},{},\"after_destroy\":{}}}",
        header("e_delete_rows", &p),
        rows as u64 * uc as u64,
        mem_json(&after_invoke, &zero),
        marks.join(","),
        mem_json(&after_destroy, &zero),
    );
}

// ── D2: Connection::call (SDK container + Rust value tree) ────────────

/// Typed inputs for Connection::call: integer parameters get
/// `RfcValue::Int` (the core rejects strings for INT targets),
/// everything else `RfcValue::String`. A key `STRUCT.FIELD` becomes a field
/// of an `RfcValue::Structure` (its fields are `RfcValue::String`).
fn typed_inputs(fd: RFC_FUNCTION_DESC_HANDLE, p: &Params) -> IndexMap<String, RfcValue> {
    let mut m = IndexMap::new();
    for (k, v) in &p.inputs {
        if let Some((s, fld)) = k.split_once('.') {
            let entry = m
                .entry(s.to_string())
                .or_insert_with(|| RfcValue::Structure(IndexMap::new()));
            if let RfcValue::Structure(sm) = entry {
                sm.insert(fld.to_string(), RfcValue::String(v.clone()));
            }
            continue;
        }
        let t = param_desc(fd, k).type_;
        let is_int = t == RFCTYPE::RFCTYPE_INT
            || t == RFCTYPE::RFCTYPE_INT1
            || t == RFCTYPE::RFCTYPE_INT2
            || t == RFCTYPE::RFCTYPE_INT8;
        let val = if is_int {
            RfcValue::Int(v.parse().expect("integer input"))
        } else {
            RfcValue::String(v.clone())
        };
        m.insert(k.clone(), val);
    }
    m
}

/// Mirrors the options crates/pyrfcx-py/src/lib.rs builds for compat=True,
/// with the facade's default config (return_import_params = False).
fn facade_compat_options() -> CompatOptions {
    CompatOptions {
        skip_empty_date_time_inputs: true,
        skip_input_length_check: true,
        pyrfc_int_setters: true,
        skip_decimal_input_check: true,
        skip_inactive_name_check: true,
        deactivate_before_inputs: true,
        read_inactive_outputs: true,
        pyrfc_date_time_setters: true,
        return_import_params: false,
    }
}

/// Heap estimate of a value tree from its shape: requested bytes and
/// allocation count. Allocator overhead per block is NOT included.
#[derive(Default)]
struct TreeEst {
    bytes: u64,
    allocs: u64,
    string_bytes: u64,
    map_bytes: u64,
}

fn hashbrown_alloc(cap: usize) -> usize {
    // hashbrown's HashTable<usize> (IndexMap's index): bucket count from
    // capacity, then buckets*8 rounded to the 16-byte group alignment, plus
    // one control byte per bucket and one trailing group of 16.
    if cap == 0 {
        return 0;
    }
    let buckets = if cap < 4 {
        4
    } else if cap < 8 {
        8
    } else {
        (cap * 8 / 7).next_power_of_two()
    };
    let data = (buckets * std::mem::size_of::<usize>()).div_ceil(16) * 16;
    data + buckets + 16
}

fn est_map(m: &IndexMap<String, RfcValue>, e: &mut TreeEst) {
    let cap = m.capacity();
    if cap > 0 {
        // IndexMap entries: Bucket { hash: usize, key: String, value: RfcValue }.
        let entries = cap * std::mem::size_of::<(usize, String, RfcValue)>();
        let idx = hashbrown_alloc(cap);
        e.map_bytes += (entries + idx) as u64;
        e.bytes += (entries + idx) as u64;
        e.allocs += 2;
    }
    for (k, v) in m {
        est_string(k.capacity(), e);
        est_value(v, e);
    }
}

fn est_string(cap: usize, e: &mut TreeEst) {
    if cap > 0 {
        e.bytes += cap as u64;
        e.string_bytes += cap as u64;
        e.allocs += 1;
    }
}

fn est_value(v: &RfcValue, e: &mut TreeEst) {
    match v {
        RfcValue::Char(s)
        | RfcValue::String(s)
        | RfcValue::Numc(s)
        | RfcValue::Decimal(s)
        | RfcValue::Date(s)
        | RfcValue::Time(s)
        | RfcValue::Temporal(s) => est_string(s.capacity(), e),
        RfcValue::Bytes(b) => {
            if b.capacity() > 0 {
                e.bytes += b.capacity() as u64;
                e.allocs += 1;
            }
        }
        RfcValue::Structure(m) => est_map(m, e),
        RfcValue::Table(rows) => {
            if rows.capacity() > 0 {
                let b = rows.capacity() * std::mem::size_of::<IndexMap<String, RfcValue>>();
                e.bytes += b as u64;
                e.map_bytes += b as u64;
                e.allocs += 1;
            }
            for r in rows {
                est_map(r, e);
            }
        }
        _ => {}
    }
}

fn d2(test: &str, compat: CompatOptions) {
    let p = params();
    let conn = connect(&p);
    let fd = func_desc(&conn, &p.func);
    let inputs = typed_inputs(fd, &p);

    // MEM_SAMPLER_MS: sampler poll interval; 0 disables the sampler.
    let interval: u64 = std::env::var("MEM_SAMPLER_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);

    let zero = mem();
    let sampler = (interval > 0).then(|| Sampler::start(Duration::from_millis(interval)));
    let t = Instant::now();
    let result = conn
        .call(&p.func, &inputs, &[], compat)
        .expect("call failed");
    let call_ms = t.elapsed().as_millis();
    let (smax_commit, smax_ws, samples) =
        sampler.map_or((zero.commit, zero.ws, 0), Sampler::finish);
    let after_call = mem();

    // Shape of the result: the measured table plus the whole tree.
    let (rows, fields) = match result.get(&p.table) {
        Some(RfcValue::Table(r)) => (r.len(), r.first().map_or(0, |m| m.len())),
        _ => (0, 0),
    };
    let mut whole = TreeEst::default();
    est_map(&result, &mut whole);
    let mut table = TreeEst::default();
    if let Some(v) = result.get(&p.table) {
        est_value(v, &mut table);
    }
    let after_estimate = mem();

    drop(result);
    let after_drop = mem();

    println!(
        "MEMJSON {{{},\"rows\":{rows},\"fields_per_row\":{fields},\"call_ms\":{call_ms},\
         \"size_of_rfcvalue\":{},\"size_of_bucket\":{},\"size_of_indexmap\":{},\
         \"tree_est_bytes\":{},\"tree_est_allocs\":{},\"table_est_bytes\":{},\"table_est_allocs\":{},\
         \"table_est_string_bytes\":{},\"table_est_map_bytes\":{},\
         \"sampler\":{{\"max_commit\":{},\"max_ws\":{},\"samples\":{samples}}},\
         \"zero_abs\":{{\"commit\":{},\"ws\":{}}},\
         \"after_call\":{},\"after_estimate\":{},\"after_drop\":{}}}",
        header(test, &p),
        std::mem::size_of::<RfcValue>(),
        std::mem::size_of::<(usize, String, RfcValue)>(),
        std::mem::size_of::<IndexMap<String, RfcValue>>(),
        whole.bytes,
        whole.allocs,
        table.bytes,
        table.allocs,
        table.string_bytes,
        table.map_bytes,
        smax_commit as i64 - zero.commit as i64,
        smax_ws as i64 - zero.ws as i64,
        zero.commit,
        zero.ws,
        mem_json(&after_call, &zero),
        mem_json(&after_estimate, &zero),
        mem_json(&after_drop, &zero),
    );
}

#[test]
#[ignore]
fn d2_call_native() {
    d2("d2_call_native", CompatOptions::default());
}

#[test]
#[ignore]
fn d2_call_compat() {
    d2("d2_call_compat", facade_compat_options());
}

// ── D3: Connection::call_chunked (SDK container, one chunk at a time) ──

/// Like D2, but through `call_chunked`: every table is taken in chunks of
/// MEM_CHUNK rows (default 4096) and each chunk is dropped at once, so no
/// value tree accumulates. `call_ms` covers the call up to the readers,
/// `read_ms` the chunk loop, `total_ms` both.
#[test]
#[ignore]
fn d3_call_chunked() {
    let p = params();
    let conn = connect(&p);
    let fd = func_desc(&conn, &p.func);
    let inputs = typed_inputs(fd, &p);
    let chunk: usize = std::env::var("MEM_CHUNK")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4096);
    let interval: u64 = std::env::var("MEM_SAMPLER_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);

    let zero = mem();
    let sampler = (interval > 0).then(|| Sampler::start(Duration::from_millis(interval)));
    let t = Instant::now();
    let mut outputs = conn
        .call_chunked(&p.func, &inputs, &[], CompatOptions::default())
        .expect("call_chunked failed");
    let call_ms = t.elapsed().as_millis();
    let after_open = mem();

    let t_read = Instant::now();
    let (mut rows, mut chunks, mut fields, mut max_chunk_bytes) = (0usize, 0usize, 0usize, 0u64);
    for (name, out) in outputs.iter_mut() {
        if let pyrfcx_core::ChunkedOutput::Table(reader) = out {
            while reader.remaining() > 0 {
                let c = reader.take_last_rows(chunk).expect("take_last_rows failed");
                if *name == p.table {
                    rows += c.len();
                    chunks += 1;
                    fields = c.first().map_or(fields, |m| m.len());
                    // The first chunk is the largest; estimating it alone
                    // keeps the walk out of read_ms.
                    if chunks == 1 {
                        let mut e = TreeEst::default();
                        est_value(&RfcValue::Table(c), &mut e);
                        max_chunk_bytes = e.bytes;
                    }
                }
            }
        }
    }
    let read_ms = t_read.elapsed().as_millis();
    let total_ms = t.elapsed().as_millis();
    let (smax_commit, smax_ws, samples) =
        sampler.map_or((zero.commit, zero.ws, 0), Sampler::finish);
    let after_drain = mem();

    drop(outputs);
    let after_drop = mem();

    println!(
        "MEMJSON {{{},\"rows\":{rows},\"fields_per_row\":{fields},\"chunk\":{chunk},\
         \"chunks\":{chunks},\"max_chunk_est_bytes\":{max_chunk_bytes},\
         \"call_ms\":{call_ms},\"read_ms\":{read_ms},\"total_ms\":{total_ms},\
         \"sampler\":{{\"max_commit\":{},\"max_ws\":{},\"samples\":{samples}}},\
         \"zero_abs\":{{\"commit\":{},\"ws\":{}}},\
         \"after_open\":{},\"after_drain\":{},\"after_drop\":{}}}",
        header("d3_call_chunked", &p),
        smax_commit as i64 - zero.commit as i64,
        smax_ws as i64 - zero.ws as i64,
        zero.commit,
        zero.ws,
        mem_json(&after_open, &zero),
        mem_json(&after_drain, &zero),
        mem_json(&after_drop, &zero),
    );
}
