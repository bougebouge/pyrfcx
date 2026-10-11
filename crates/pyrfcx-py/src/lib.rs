mod compat_input;

use std::collections::HashMap;
use std::sync::Mutex;

use indexmap::IndexMap;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyNone, PyString};

type PyObject = Py<pyo3::PyAny>;

use pyrfcx_core::{ChunkedOutput, CompatOptions, ConnectionParams, RfcError, RfcValue};

/// Convert a `pyrfcx_core::RfcError` to the matching Python exception from
/// `pyrfcx._exceptions` and return it as a `PyErr`.
fn rfc_error_to_pyerr(py: Python<'_>, err: &RfcError) -> PyErr {
    let exceptions = match py.import("pyrfcx._exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };

    match err {
        RfcError::SdkNotFound(msg) => {
            let cls = match exceptions.getattr("SdkNotFoundError") {
                Ok(c) => c,
                Err(e) => return e,
            };
            match cls.call1((msg.as_str(),)) {
                Ok(exc) => PyErr::from_value(exc),
                Err(e) => e,
            }
        }
        RfcError::InvalidInput(msg) => pyo3::exceptions::PyValueError::new_err(msg.clone()),
        RfcError::InvalidType(msg) => pyo3::exceptions::PyTypeError::new_err(msg.clone()),
        _ => {
            let info = err
                .info()
                .expect("non-SdkNotFound/InvalidInput variant always has info");
            let cls_name = match err {
                RfcError::CommunicationFailure(_) => "CommunicationError",
                RfcError::LogonFailure(_) => "LogonError",
                RfcError::AbapRuntimeFailure(_) => "AbapRuntimeError",
                RfcError::AbapException(_) => "AbapApplicationError",
                RfcError::Other(_) => "RfcError",
                RfcError::SdkNotFound(_) | RfcError::InvalidInput(_) | RfcError::InvalidType(_) => {
                    unreachable!()
                }
            };
            let cls = match exceptions.getattr(cls_name) {
                Ok(c) => c,
                Err(e) => return e,
            };

            let kwargs = PyDict::new(py);
            kwargs.set_item("code", info.code).unwrap();
            kwargs.set_item("group", info.group).unwrap();
            kwargs.set_item("key", &info.key).unwrap();
            kwargs
                .set_item("abap_msg_class", &info.abap_msg_class)
                .unwrap();
            kwargs
                .set_item("abap_msg_type", &info.abap_msg_type)
                .unwrap();
            kwargs
                .set_item("abap_msg_number", &info.abap_msg_number)
                .unwrap();
            kwargs.set_item("abap_msg_v1", &info.abap_msg_v1).unwrap();
            kwargs.set_item("abap_msg_v2", &info.abap_msg_v2).unwrap();
            kwargs.set_item("abap_msg_v3", &info.abap_msg_v3).unwrap();
            kwargs.set_item("abap_msg_v4", &info.abap_msg_v4).unwrap();
            kwargs.set_item("sdk_function", info.sdk_function).unwrap();

            match cls.call((&info.message,), Some(&kwargs)) {
                Ok(exc) => PyErr::from_value(exc),
                Err(e) => e,
            }
        }
    }
}

// ── Input conversion: Python → RfcValue ────────────────────────────────
// Dispatch order:
//   dict → Structure, list → Table, bytes → Bytes,
//   bool → TypeError (before int!), str → String,
//   int → Int(i128), float → Float, Decimal → Decimal(str(d)),
//   datetime.date → Date, datetime.time → Time,
//   anything else (incl. None) → TypeError.

/// How a caller's name (function, parameter, field, inactive) becomes the
/// name handed to the core. Native passes it unchanged, so the core's NUL
/// check sees it; compat uses `compat_input::pyrfc_sdk_name`.
type NameRule = fn(&str) -> &str;

fn native_name(name: &str) -> &str {
    name
}

fn py_to_rfcvalue(
    py: Python<'_>,
    obj: &Bound<'_, pyo3::PyAny>,
    name_rule: NameRule,
) -> PyResult<RfcValue> {
    if let Ok(dict) = obj.cast::<PyDict>() {
        let mut map = IndexMap::with_capacity(dict.len());
        for (k, v) in dict.iter() {
            let key: String = k.extract()?;
            let val = py_to_rfcvalue(py, &v, name_rule)?;
            map.insert(name_rule(&key).to_owned(), val);
        }
        return Ok(RfcValue::Structure(map));
    }

    if let Ok(list) = obj.cast::<PyList>() {
        let mut rows = Vec::with_capacity(list.len());
        for item in list.iter() {
            let dict = item
                .cast::<PyDict>()
                .map_err(|_| pyo3::exceptions::PyTypeError::new_err("table rows must be dicts"))?;
            let mut map = IndexMap::with_capacity(dict.len());
            for (k, v) in dict.iter() {
                let key: String = k.extract()?;
                let val = py_to_rfcvalue(py, &v, name_rule)?;
                map.insert(name_rule(&key).to_owned(), val);
            }
            rows.push(map);
        }
        return Ok(RfcValue::Table(rows));
    }

    if let Ok(b) = obj.cast::<PyBytes>() {
        return Ok(RfcValue::Bytes(b.as_bytes().to_vec()));
    }

    // bytearray → Bytes: unambiguous byte carrier, accepted by pyrfc too
    // (fillBytes takes any buffer). A superset of the dispatch below.
    if let Ok(ba) = obj.cast::<pyo3::types::PyByteArray>() {
        return Ok(RfcValue::Bytes(ba.to_vec()));
    }

    // bool → TypeError (MUST come before int — bool subclasses int)
    if obj.is_instance_of::<PyBool>() {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "bool is not a valid RFC value; use int or str",
        ));
    }

    if let Ok(s) = obj.cast::<PyString>() {
        return Ok(RfcValue::String(s.to_string()));
    }

    if obj.is_instance_of::<PyInt>() {
        // Extract numerically — subclass-safe (str(IntEnum) is
        // version-dependent). Beyond-i128 propagates Python's natural
        // OverflowError; i128 covers every representable integer target.
        let n: i128 = obj.extract()?;
        return Ok(RfcValue::Int(n));
    }

    if let Ok(f) = obj.cast::<PyFloat>() {
        return Ok(RfcValue::Float(f.value()));
    }

    // decimal.Decimal → Decimal(str(d))
    let decimal_mod = py.import("decimal")?;
    let decimal_cls = decimal_mod.getattr("Decimal")?;
    if obj.is_instance(&decimal_cls)? {
        let s: String = obj.str()?.to_string();
        return Ok(RfcValue::Decimal(s));
    }

    // datetime types — check time before date (datetime subclasses date)
    let dt_mod = py.import("datetime")?;
    let time_cls = dt_mod.getattr("time")?;
    let date_cls = dt_mod.getattr("date")?;

    if obj.is_instance(&time_cls)? {
        let s: String = obj.call_method1("strftime", ("%H%M%S",))?.extract()?;
        return Ok(RfcValue::Time(s));
    }

    if obj.is_instance(&date_cls)? {
        let s: String = obj.call_method1("strftime", ("%Y%m%d",))?.extract()?;
        return Ok(RfcValue::Date(s));
    }

    Err(pyo3::exceptions::PyTypeError::new_err(format!(
        "unsupported type for RFC input: {}",
        obj.get_type().name()?
    )))
}

// ── Output conversion: RfcValue → Python ───────────────────────────────

/// The dict keys of one result conversion: each field name becomes one
/// Python `str`, shared by every dict that has the field — every row of
/// every table, structures and nested tables included — instead of a new
/// `str` per field per row. Lives for one conversion only: no global state.
/// Owns its names, because the rows they come from are dropped chunk by
/// chunk while the conversion goes on.
struct KeyCache<'py> {
    py: Python<'py>,
    keys: HashMap<String, Bound<'py, PyString>>,
}

/// One table's keys in column order. The rows of a table repeat the same
/// fields in the same order, so slot `j` almost always holds the key of
/// field `j`: a string compare per field, no hash lookup.
type RowKeys<'py> = Vec<(String, Bound<'py, PyString>)>;

impl<'py> KeyCache<'py> {
    fn new(py: Python<'py>) -> Self {
        Self {
            py,
            keys: HashMap::new(),
        }
    }

    fn key(&mut self, name: &str) -> Bound<'py, PyString> {
        if let Some(key) = self.keys.get(name) {
            return key.clone();
        }
        let key = PyString::new(self.py, name);
        self.keys.insert(name.to_owned(), key.clone());
        key
    }

    /// The key of field `j` of a table row, from the table's column slots.
    /// A row whose fields differ from the first row's falls back to `key`.
    fn row_key(
        &mut self,
        row_keys: &mut RowKeys<'py>,
        j: usize,
        name: &str,
    ) -> Bound<'py, PyString> {
        if let Some((slot_name, key)) = row_keys.get(j)
            && slot_name == name
        {
            return key.clone();
        }
        let key = self.key(name);
        if j == row_keys.len() {
            row_keys.push((name.to_owned(), key.clone()));
        }
        key
    }
}

/// One table row as a Python dict. `compat` is `None` for the native profile.
fn row_to_py<'py>(
    py: Python<'py>,
    row: &IndexMap<String, RfcValue>,
    row_keys: &mut RowKeys<'py>,
    keys: &mut KeyCache<'py>,
    compat: Option<CompatOutputConfig>,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    for (j, (k, v)) in row.iter().enumerate() {
        let key = keys.row_key(row_keys, j, k);
        let obj = match compat {
            Some(cfg) => rfcvalue_to_py_compat(py, v, cfg, keys)?,
            None => rfcvalue_to_py_native(py, v, keys)?,
        };
        dict.set_item(key, obj)?;
    }
    Ok(dict)
}

/// Native profile: full-fidelity Python objects.
fn rfcvalue_to_py_native<'py>(
    py: Python<'py>,
    value: &RfcValue,
    keys: &mut KeyCache<'py>,
) -> PyResult<PyObject> {
    match value {
        // Strip trailing U+0020 only — the SDK's pad character.
        RfcValue::Char(s) => {
            let stripped = s.trim_end_matches(' ');
            Ok(stripped.into_pyobject(py)?.into_any().unbind())
        }

        RfcValue::String(s)
        | RfcValue::Numc(s)
        | RfcValue::Date(s)
        | RfcValue::Time(s)
        | RfcValue::Temporal(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),

        RfcValue::Decimal(s) => {
            let decimal_mod = py.import("decimal")?;
            let decimal_cls = decimal_mod.getattr("Decimal")?;
            // The SDK can return an empty string (rc=OK) for a BCD
            // field holding invalid packed content (seen live: never-filled
            // license data). Decimal("") is unconstructible; NaN is the
            // truthful "not a number" — no invented value. (The compat
            // profile instead replicates pyrfc's crash: InvalidOperation.)
            let src = if s.is_empty() { "NaN" } else { s.as_str() };
            let obj = decimal_cls.call1((src,))?;
            Ok(obj.unbind())
        }

        RfcValue::Float(f) => Ok(f.into_pyobject(py)?.into_any().unbind()),
        // Int is an input-side carrier — never read from the SDK,
        // but convert faithfully for totality.
        RfcValue::Int(n) => Ok(n.into_pyobject(py)?.into_any().unbind()),
        RfcValue::Int1(n) => Ok(n.into_pyobject(py)?.into_any().unbind()),
        RfcValue::Int2(n) => Ok(n.into_pyobject(py)?.into_any().unbind()),
        RfcValue::Int4(n) => Ok(n.into_pyobject(py)?.into_any().unbind()),
        RfcValue::Int8(n) => Ok(n.into_pyobject(py)?.into_any().unbind()),

        RfcValue::Bytes(b) => Ok(PyBytes::new(py, b).into_any().unbind()),

        RfcValue::Structure(map) => {
            let dict = PyDict::new(py);
            for (k, v) in map {
                dict.set_item(keys.key(k), rfcvalue_to_py_native(py, v, keys)?)?;
            }
            Ok(dict.into_any().unbind())
        }

        RfcValue::Table(rows) => {
            let list = PyList::empty(py);
            let mut row_keys = RowKeys::new();
            for row in rows {
                list.append(row_to_py(py, row, &mut row_keys, keys, None)?)?;
            }
            Ok(list.into_any().unbind())
        }
    }
}

/// Strip trailing ASCII whitespace, the pyrfc rstrip set:
/// space, tab, newline, carriage return, vertical tab, form feed.
fn compat_rstrip(s: &str) -> &str {
    s.trim_end_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c'])
}

/// COMPAT-ONLY: pyrfc 3.4's output `config` flags. NOT SDK concepts; they
/// exist solely for pyrfc retrocompatibility.
#[derive(Debug, Clone, Copy)]
struct CompatOutputConfig {
    /// pyrfc's `rstrip`: strip CHAR and BCD/DECF output. STRING and NUMC are
    /// stripped always, DATE/TIME never, whatever this flag says.
    rstrip: bool,
    /// pyrfc's `dtime`: DATE/TIME output as `datetime.date`/`datetime.time`.
    dtime: bool,
}

/// pyrfc's `datetime.strptime(value, fmt)`, called through Python so that
/// what it accepts and the `ValueError` it raises are exactly pyrfc's.
fn compat_strptime<'py>(py: Python<'py>, value: &str, fmt: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import("datetime")?
        .getattr("datetime")?
        .call_method1("strptime", (value, fmt))
}

/// Compat profile: pyrfc-compatible projections.
fn rfcvalue_to_py_compat<'py>(
    py: Python<'py>,
    value: &RfcValue,
    cfg: CompatOutputConfig,
    keys: &mut KeyCache<'py>,
) -> PyResult<PyObject> {
    match value {
        RfcValue::Char(s) => {
            let out = if cfg.rstrip { compat_rstrip(s) } else { s };
            Ok(out.into_pyobject(py)?.into_any().unbind())
        }
        RfcValue::String(s) => Ok(compat_rstrip(s).into_pyobject(py)?.into_any().unbind()),
        RfcValue::Numc(s) => Ok(compat_rstrip(s).into_pyobject(py)?.into_any().unbind()),
        // pyrfc rstrips the SDK string (when `rstrip` is on), then wraps in
        // decimal.Decimal, as pyrfc does. An empty SDK decimal string
        // crashes with InvalidOperation here — exactly what pyrfc 3.4 does;
        // do not soften it.
        RfcValue::Decimal(s) => {
            let decimal_mod = py.import("decimal")?;
            let decimal_cls = decimal_mod.getattr("Decimal")?;
            let src = if cfg.rstrip { compat_rstrip(s) } else { s };
            let obj = decimal_cls.call1((src,))?;
            Ok(obj.unbind())
        }
        RfcValue::Float(f) => Ok(f.into_pyobject(py)?.into_any().unbind()),
        // Int is an input-side carrier — never read from the SDK.
        RfcValue::Int(n) => Ok(n.into_pyobject(py)?.into_any().unbind()),
        RfcValue::Int1(n) => Ok(n.into_pyobject(py)?.into_any().unbind()),
        RfcValue::Int2(n) => Ok(n.into_pyobject(py)?.into_any().unbind()),
        RfcValue::Int4(n) => Ok(n.into_pyobject(py)?.into_any().unbind()),
        RfcValue::Int8(n) => Ok(n.into_pyobject(py)?.into_any().unbind()),

        // Initial DATE "00000000" → "" (K4), or None with `dtime`. Any other
        // value under `dtime` goes to strptime; a blank or invalid date
        // raises its ValueError, as in pyrfc.
        RfcValue::Date(s) => {
            let initial = s == "00000000" || s.is_empty();
            if cfg.dtime {
                if initial {
                    return Ok(py.None());
                }
                let dt = compat_strptime(py, s, "%Y%m%d")?;
                return Ok(dt.call_method0("date")?.unbind());
            }
            let out = if initial { "" } else { s.as_str() };
            Ok(out.into_pyobject(py)?.into_any().unbind())
        }

        // TIME has no initial sentinel: "000000" is midnight under `dtime`.
        RfcValue::Time(s) => {
            if cfg.dtime {
                if s.is_empty() {
                    return Ok(py.None());
                }
                let dt = compat_strptime(py, s, "%H%M%S")?;
                return Ok(dt.call_method0("time")?.unbind());
            }
            Ok(s.into_pyobject(py)?.into_any().unbind())
        }

        // UTCLONG: comma at index 19 → dot
        RfcValue::Temporal(s) => {
            let out = if s.len() > 19 {
                let mut chars: Vec<char> = s.chars().collect();
                if chars.get(19) == Some(&',') {
                    chars[19] = '.';
                }
                chars.into_iter().collect::<String>()
            } else {
                s.clone()
            };
            Ok(out.into_pyobject(py)?.into_any().unbind())
        }

        RfcValue::Bytes(b) => Ok(PyBytes::new(py, b).into_any().unbind()),

        RfcValue::Structure(map) => {
            let dict = PyDict::new(py);
            for (k, v) in map {
                dict.set_item(keys.key(k), rfcvalue_to_py_compat(py, v, cfg, keys)?)?;
            }
            Ok(dict.into_any().unbind())
        }

        RfcValue::Table(rows) => {
            let list = PyList::empty(py);
            let mut row_keys = RowKeys::new();
            for row in rows {
                list.append(row_to_py(py, row, &mut row_keys, keys, Some(cfg))?)?;
            }
            Ok(list.into_any().unbind())
        }
    }
}

// ── Chunked results: top-level tables leave the SDK chunk by chunk ──────

/// Rows taken from a top-level table per GIL-free step: large enough that
/// the GIL hand-offs cost nothing, small enough that the Rust copy of a
/// chunk is negligible beside the Python result.
const CHUNK_ROWS: usize = 256;

type Row = IndexMap<String, RfcValue>;

/// One output of `call_chunked`, with each table's last chunk already
/// taken inside the GIL-free section of the call.
enum PendingOutput {
    Value(RfcValue),
    Table {
        reader: pyrfcx_core::TableReader,
        last_rows: Vec<Row>,
    },
}

/// Take the last chunk of every table now, while the GIL is released: a
/// table that fits in one chunk then needs no further GIL hand-off.
#[allow(clippy::result_large_err)] // RfcError is the core's error type, same allowance as pyrfcx-core
fn take_first_chunks(
    outputs: IndexMap<String, ChunkedOutput>,
) -> Result<Vec<(String, PendingOutput)>, RfcError> {
    outputs
        .into_iter()
        .map(|(name, output)| {
            let pending = match output {
                ChunkedOutput::Value(v) => PendingOutput::Value(v),
                ChunkedOutput::Table(mut reader) => {
                    let last_rows = reader.take_last_rows(CHUNK_ROWS)?;
                    PendingOutput::Table { reader, last_rows }
                }
            };
            Ok((name, pending))
        })
        .collect()
}

/// Convert one top-level table, taking the remaining chunks with the GIL
/// released. Each chunk is placed at its final indices and dropped before
/// the next is taken, so the Rust rows never pile up beside the list.
#[allow(clippy::result_large_err)] // the detached closure returns Result<_, RfcError>
fn table_to_py<'py>(
    py: Python<'py>,
    mut reader: pyrfcx_core::TableReader,
    last_rows: Vec<Row>,
    keys: &mut KeyCache<'py>,
    compat: Option<CompatOutputConfig>,
) -> PyResult<Bound<'py, PyList>> {
    let list = PyList::new(py, std::iter::repeat_n(PyNone::get(py), reader.row_count()))?;
    let mut row_keys = RowKeys::new();
    // Chunks arrive last rows first. Converting row by row in order raises
    // the error of the earliest bad row, so a later chunk's error replaces
    // an earlier one; once one is kept, rows are no longer stored.
    let mut row_error: Option<PyErr> = None;
    let mut chunk = last_rows;
    loop {
        let start = reader.remaining();
        for (i, row) in chunk.iter().enumerate() {
            match row_to_py(py, row, &mut row_keys, keys, compat) {
                Ok(dict) if row_error.is_none() => list.set_item(start + i, dict)?,
                Ok(_) => {}
                Err(e) => {
                    row_error = Some(e);
                    break;
                }
            }
        }
        drop(chunk);
        if reader.remaining() == 0 {
            break;
        }
        // A read error wins over a conversion error, as it did when the
        // whole table was read before any row was converted.
        chunk = py
            .detach(|| reader.take_last_rows(CHUNK_ROWS))
            .map_err(|e| rfc_error_to_pyerr(py, &e))?;
    }
    match row_error {
        Some(e) => Err(e),
        None => Ok(list),
    }
}

/// `compat` is `None` for the native profile. On error every reader is
/// dropped with the iterator, which frees the SDK container.
fn result_to_py(
    py: Python<'_>,
    outputs: Vec<(String, PendingOutput)>,
    compat: Option<CompatOutputConfig>,
) -> PyResult<PyObject> {
    let dict = PyDict::new(py);
    let mut keys = KeyCache::new(py);
    for (name, output) in outputs {
        let obj = match output {
            PendingOutput::Value(v) => match compat {
                Some(cfg) => rfcvalue_to_py_compat(py, &v, cfg, &mut keys)?,
                None => rfcvalue_to_py_native(py, &v, &mut keys)?,
            },
            PendingOutput::Table { reader, last_rows } => {
                table_to_py(py, reader, last_rows, &mut keys, compat)?
                    .into_any()
                    .unbind()
            }
        };
        dict.set_item(keys.key(&name), obj)?;
    }
    Ok(dict.into_any().unbind())
}

/// One parameter or field type as a dict for `pyrfcx.Connection.describe_function`.
/// Type and direction codes stay ints; pure Python maps them to enums.
fn field_desc_to_py<'py>(
    py: Python<'py>,
    d: &Bound<'py, PyDict>,
    desc: &pyrfcx_core::FieldDesc,
) -> PyResult<()> {
    d.set_item("type", desc.rfctype.code())?;
    d.set_item("nuc_length", desc.nuc_length)?;
    d.set_item("uc_length", desc.uc_length)?;
    d.set_item("decimals", desc.decimals)?;
    match &desc.type_info {
        Some(info) => {
            let t = PyDict::new(py);
            t.set_item("name", &info.name)?;
            t.set_item("nuc_length", info.nuc_length)?;
            t.set_item("uc_length", info.uc_length)?;
            let fields = PyList::empty(py);
            for f in &desc.fields {
                let fd = PyDict::new(py);
                fd.set_item("name", &f.name)?;
                fd.set_item("nuc_offset", f.nuc_offset)?;
                fd.set_item("uc_offset", f.uc_offset)?;
                field_desc_to_py(py, &fd, &f.desc)?;
                fields.append(fd)?;
            }
            t.set_item("fields", fields)?;
            d.set_item("type_description", t)?;
        }
        None => d.set_item("type_description", py.None())?,
    }
    Ok(())
}

fn func_desc_to_py<'py>(
    py: Python<'py>,
    desc: &pyrfcx_core::FuncDesc,
) -> PyResult<Bound<'py, PyDict>> {
    let params = PyList::empty(py);
    for p in &desc.params {
        let d = PyDict::new(py);
        d.set_item("name", &p.name)?;
        d.set_item("direction", p.direction.code())?;
        d.set_item("default_value", &p.default_value)?;
        d.set_item("text", &p.text)?;
        d.set_item("optional", p.optional)?;
        field_desc_to_py(py, &d, &p.desc)?;
        params.append(d)?;
    }
    let f = PyDict::new(py);
    f.set_item("name", &desc.name)?;
    f.set_item("parameters", params)?;
    Ok(f)
}

/// Holds a connection taken out of `Connection::inner` and puts it back on
/// drop, on every path — error and panic included.
struct TakenConnection<'a> {
    slot: &'a Mutex<Option<pyrfcx_core::Connection>>,
    conn: Option<pyrfcx_core::Connection>,
}

impl Drop for TakenConnection<'_> {
    fn drop(&mut self) {
        let mut guard = self.slot.lock().expect("connection mutex poisoned");
        *guard = self.conn.take();
    }
}

#[pyclass]
struct Connection {
    inner: Mutex<Option<pyrfcx_core::Connection>>,
    /// Cancels an in-flight call from another thread (`RfcCancel`). Held
    /// separately from `inner` because during `call` the core connection
    /// is taken out of the mutex and moved into the GIL-free closure —
    /// exactly when a canceller needs to reach the handle.
    cancel: pyrfcx_core::CancelToken,
}

impl Connection {
    /// Fetch a function's signature, releasing the GIL: the first fetch per
    /// function is a network round-trip (afterwards the SDK serves it from
    /// its repository cache and only the metadata walk remains).
    ///
    /// The connection is taken out of the mutex and restored on every path,
    /// error included — `MutexGuard` is not `Send`, so it cannot be held
    /// across `detach`.
    fn describe(&self, py: Python<'_>, function_name: &str) -> PyResult<pyrfcx_core::FuncDesc> {
        let mut guard = self.inner.lock().expect("connection mutex poisoned");
        let conn = guard
            .take()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("connection is closed"))?;
        drop(guard);

        let fname = function_name.to_string();
        let (conn, result) = py.detach(move || {
            let r = conn.describe_function(&fname);
            (conn, r)
        });

        let mut guard = self.inner.lock().expect("connection mutex poisoned");
        *guard = Some(conn);
        drop(guard);

        result.map_err(|e| rfc_error_to_pyerr(py, &e))
    }
}

#[pymethods]
impl Connection {
    #[new]
    fn new(py: Python<'_>, params: HashMap<String, String>) -> PyResult<Self> {
        let mut cp = ConnectionParams::new();
        for (k, v) in &params {
            cp = cp.set(k.as_str(), v.as_str());
        }

        match pyrfcx_core::Connection::open(&cp) {
            Ok(c) => Ok(Connection {
                cancel: c.cancel_token(),
                inner: Mutex::new(Some(c)),
            }),
            Err(e) => Err(rfc_error_to_pyerr(py, &e)),
        }
    }

    /// Cancel the RFC call currently executing on this connection
    /// (`RfcCancel`) — called from a different thread than the one blocked
    /// in `call`. Per the SDK, a successful cancel also closes the
    /// connection.
    ///
    /// GIL released: RfcCancel talks to the backend, and the token doesn't
    /// need the `inner` mutex (the in-flight call holds the connection).
    #[allow(clippy::result_large_err)] // closure returns Result<_, RfcError>, same allowance as pyrfcx-core
    fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        let token = self.cancel.clone();
        py.detach(move || token.cancel())
            .map_err(|e| rfc_error_to_pyerr(py, &e))
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let mut guard = self.inner.lock().expect("connection mutex poisoned");
        match guard.as_mut() {
            Some(conn) => match conn.close() {
                Ok(()) => Ok(()),
                Err(e) => Err(rfc_error_to_pyerr(py, &e)),
            },
            None => Ok(()),
        }
    }

    /// Read the connection's attributes (`RfcGetConnectionAttributes`).
    ///
    /// Returns a plain dict keyed by the core's snake_case field names; the
    /// typed `ConnectionAttributes` is assembled in pure Python, which also
    /// supplies `dest` (the SDK reports it empty — see the core's docs).
    ///
    /// No GIL release: unlike `call`, this is served from the connection's
    /// local state, not a network round-trip.
    fn attributes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let guard = self.inner.lock().expect("connection mutex poisoned");
        let conn = guard
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("connection is closed"))?;
        let a = conn.attributes().map_err(|e| rfc_error_to_pyerr(py, &e))?;

        let d = PyDict::new(py);
        d.set_item("dest", a.dest)?;
        d.set_item("host", a.host)?;
        d.set_item("partner_host", a.partner_host)?;
        d.set_item("sys_number", a.sys_number)?;
        d.set_item("sys_id", a.sys_id)?;
        d.set_item("client", a.client)?;
        d.set_item("user", a.user)?;
        d.set_item("language", a.language)?;
        d.set_item("trace", a.trace)?;
        d.set_item("iso_language", a.iso_language)?;
        d.set_item("codepage", a.codepage)?;
        d.set_item("partner_codepage", a.partner_codepage)?;
        d.set_item("rfc_role", a.rfc_role)?;
        d.set_item("type", a.r#type)?;
        d.set_item("partner_type", a.partner_type)?;
        d.set_item("rel", a.rel)?;
        d.set_item("partner_rel", a.partner_rel)?;
        d.set_item("kernel_rel", a.kernel_rel)?;
        d.set_item("cpic_conv_id", a.cpic_conv_id)?;
        d.set_item("prog_name", a.prog_name)?;
        d.set_item("partner_bytes_per_char", a.partner_bytes_per_char)?;
        d.set_item("partner_system_codepage", a.partner_system_codepage)?;
        d.set_item("partner_ip", a.partner_ip)?;
        d.set_item("partner_ipv6", a.partner_ipv6)?;
        Ok(d)
    }

    /// Describe a function module's signature (`RfcGetFunctionDesc` plus
    /// the descriptor walk). Returns nested dicts; `pyrfcx.Connection`
    /// builds the typed dataclasses in pure Python.
    ///
    /// The name goes to the SDK unchanged; the SDK lookup is case-sensitive
    /// (a lower-case name gives `FU_NOT_FOUND`).
    ///
    /// GIL released: the first fetch per function is a network round-trip.
    fn describe_function<'py>(
        &self,
        py: Python<'py>,
        function_name: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        let desc = self.describe(py, function_name)?;
        func_desc_to_py(py, &desc)
    }

    /// Whether the SDK still holds the connection
    /// (`RfcIsConnectionHandleValid`). `False` once closed. This asks the
    /// SDK about the handle; `pyrfcx.Connection.closed` is what pyrfcx knows.
    ///
    /// No GIL release: the SDK checks local state, no network round-trip.
    fn is_valid(&self, py: Python<'_>) -> PyResult<bool> {
        let guard = self.inner.lock().expect("connection mutex poisoned");
        match guard.as_ref() {
            Some(conn) => conn.is_valid().map_err(|e| rfc_error_to_pyerr(py, &e)),
            None => Ok(false),
        }
    }

    /// Ping the communication partner (`RfcPing`).
    ///
    /// A network round-trip, so the GIL is released like `call` (take the
    /// connection out of the mutex, `detach`, restore) — unlike
    /// `attributes`, which is served from local state.
    fn ping(&self, py: Python<'_>) -> PyResult<()> {
        let mut guard = self.inner.lock().expect("connection mutex poisoned");
        let conn = guard
            .take()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("connection is closed"))?;
        drop(guard);

        let (conn, result) = py.detach(move || {
            let r = conn.ping();
            (conn, r)
        });

        let mut guard = self.inner.lock().expect("connection mutex poisoned");
        *guard = Some(conn);
        drop(guard);

        result.map_err(|e| rfc_error_to_pyerr(py, &e))
    }

    /// `check_date`/`check_time` mirror pyrfc's config bitmask flags and
    /// apply only when `compat=True`; the native path ignores them.
    ///
    /// `not_requested` is COMPAT-ONLY (pyrfc's `options["not_requested"]`):
    /// names to deactivate that may ALSO carry a value in `params` — pyrfc
    /// deactivates first and still fills the value. Native callers use the
    /// `INACTIVE` sentinel instead.
    ///
    /// `rstrip`, `dtime` and `return_import_params` are COMPAT-ONLY too:
    /// pyrfc's output `config` flags. The native path ignores them.
    #[pyo3(signature = (
        function_name,
        params,
        compat=false,
        check_date=true,
        check_time=true,
        not_requested=Vec::new(),
        rstrip=true,
        dtime=false,
        return_import_params=false,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn call(
        &self,
        py: Python<'_>,
        function_name: &str,
        params: &Bound<'_, PyDict>,
        compat: bool,
        check_date: bool,
        check_time: bool,
        not_requested: Vec<String>,
        rstrip: bool,
        dtime: bool,
        return_import_params: bool,
    ) -> PyResult<PyObject> {
        // Split out INACTIVE sentinels first — they are per-call
        // control (RfcSetParameterActive), not data, so neither the compat
        // validation nor the value conversion should ever see them.
        // COMPAT-ONLY: under pyrfc the SDK reads every name up to its first
        // NUL, so compat cuts the names the same way; native hands them on
        // whole and the core rejects an embedded NUL.
        let name_rule: NameRule = if compat {
            compat_input::pyrfc_sdk_name
        } else {
            native_name
        };
        let function_name = name_rule(function_name);

        let inactive_sentinel = py.import("pyrfcx._sentinels")?.getattr("INACTIVE")?;
        let data_params = PyDict::new(py);
        let mut inactive: Vec<String> = Vec::new();
        for (k, v) in params.iter() {
            if v.is(&inactive_sentinel) {
                inactive.push(name_rule(&k.extract::<String>()?).to_owned());
            } else {
                data_params.set_item(k, v)?;
            }
        }
        inactive.extend(not_requested.iter().map(|n| name_rule(n).to_owned()));

        // COMPAT-ONLY: mirror pyrfc's client-side input validation
        // before anything is converted. Needs the target types, which only
        // the core can read, and the caller's live Python objects, which
        // only this layer holds — so it runs here, against the core's
        // descriptor tree.
        //
        // pyrfc deactivates `not_requested` names BEFORE it fills (and
        // validates) any value, so an unknown name there must win over any
        // input error: drop the inputs and let the core's deactivation
        // raise the SDK's own error.
        let mut unknown_inactive = false;
        let compat_desc = if compat {
            let desc = self.describe(py, function_name)?;
            unknown_inactive = inactive.iter().any(|name| desc.param(name).is_none());
            if !unknown_inactive {
                let cfg = compat_input::CompatInputConfig {
                    check_date,
                    check_time,
                };
                compat_input::validate_inputs(py, &data_params, &desc, cfg)?;
            }
            Some(desc)
        } else {
            None
        };
        if unknown_inactive {
            data_params.clear();
        }

        let mut inputs = IndexMap::with_capacity(data_params.len());
        for (k, v) in data_params.iter() {
            let key: String = k.extract()?;
            let val = py_to_rfcvalue(py, &v, name_rule)?;
            inputs.insert(name_rule(&key).to_owned(), val);
        }
        // COMPAT-ONLY: pyrfc sends CHAR/STRING/UTCLONG text only up to its
        // first NUL; native sends the whole value.
        if let Some(desc) = &compat_desc {
            compat_input::pyrfc_cut_text_values(&mut inputs, desc);
        }

        let compat_opts = if compat {
            CompatOptions {
                skip_empty_date_time_inputs: true,
                skip_input_length_check: true,
                pyrfc_int_setters: true,
                // The compat pre-pass validates decimals with pyrfc's grammar
                // before the core; the core's own decimal check would be
                // redundant (and could diverge), so skip it in compat.
                skip_decimal_input_check: true,
                // pyrfc deactivates before filling and never asks the SDK
                // whether an output is active.
                deactivate_before_inputs: true,
                read_inactive_outputs: true,
                pyrfc_date_time_setters: true,
                return_import_params,
            }
        } else {
            CompatOptions::default()
        };

        // Take the connection out so we can move it into the GIL-free
        // closure. MutexGuard isn't Send, so we must drop it first.
        let mut guard = self.inner.lock().expect("connection mutex poisoned");
        let conn = guard
            .take()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("connection is closed"))?;
        drop(guard);

        // Own the function name so the closure is 'static + Send.
        let fname = function_name.to_string();

        // Release the GIL during the SDK call (PyO3 0.29: detach).
        let (conn, result) = py.detach(move || {
            let r = conn
                .call_chunked(&fname, &inputs, &inactive, compat_opts)
                .and_then(take_first_chunks);
            (conn, r)
        });

        // The connection stays out until every table is converted: the
        // tables are still being read, and a read phase has always run
        // with the connection taken out. Put back on drop, on every path.
        let _taken = TakenConnection {
            slot: &self.inner,
            conn: Some(conn),
        };

        match result {
            Ok(outputs) => {
                let profile = compat.then_some(CompatOutputConfig { rstrip, dtime });
                result_to_py(py, outputs, profile)
            }
            Err(e) => Err(rfc_error_to_pyerr(py, &e)),
        }
    }
}

/// The loaded SDK's own version, via `RfcGetVersion`:
/// `(major, minor, patch_level, description)`.
///
/// This reports the library actually loaded, not the one we hoped for — the
/// loader falls back to a bare `sapnwrfc.dll` when `SAPNWRFC_HOME` is unset,
/// and Windows will then resolve it from the ambient DLL search order. Tests
/// pin the expected SDK and assert against this; fact ledgers are stamped
/// with it.
#[pyfunction]
fn sdk_version(py: Python<'_>) -> PyResult<(u32, u32, u32, String)> {
    pyrfcx_core::ffi::get_version().map_err(|e| rfc_error_to_pyerr(py, &e))
}

/// Set the directory where the SDK looks for `sapnwrfc.ini`
/// (`RfcSetIniPath`). Process-wide; the traps are documented on
/// `pyrfcx_core::ffi::set_ini_path`. An embedded NUL raises `ValueError`
/// before any SDK call.
#[pyfunction]
fn set_ini_path(py: Python<'_>, directory: &str) -> PyResult<()> {
    pyrfcx_core::ffi::set_ini_path(directory).map_err(|e| rfc_error_to_pyerr(py, &e))
}

/// Read `sapnwrfc.ini` again (`RfcReloadIniFile`). Process-wide; the
/// traps are documented on `pyrfcx_core::ffi::reload_ini_file`.
#[pyfunction]
fn reload_ini_file(py: Python<'_>) -> PyResult<()> {
    pyrfcx_core::ffi::reload_ini_file().map_err(|e| rfc_error_to_pyerr(py, &e))
}

/// Set the path of the SAP crypto library for TLS (`RfcLoadCryptoLibrary`).
/// Process-wide and final after the first call, even a failed one; the
/// traps are documented on `pyrfcx_core::ffi::load_crypto_library`. An
/// embedded NUL raises `ValueError` before any SDK call.
#[pyfunction]
fn load_crypto_library(py: Python<'_>, path: &str) -> PyResult<()> {
    pyrfcx_core::ffi::load_crypto_library(path).map_err(|e| rfc_error_to_pyerr(py, &e))
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Connection>()?;
    m.add_function(wrap_pyfunction!(sdk_version, m)?)?;
    m.add_function(wrap_pyfunction!(set_ini_path, m)?)?;
    m.add_function(wrap_pyfunction!(reload_ini_file, m)?)?;
    m.add_function(wrap_pyfunction!(load_crypto_library, m)?)?;
    Ok(())
}
