// COMPAT-ONLY: a mirror of pyrfc 3.4's `fillVariable` input
// validation. NOT an SDK concept and NOT part of the native pyrfcx API —
// it exists solely so the pyrfc facade rejects the same inputs, with the
// same `TypeError.args`, that pyrfc 3.4 rejected.
//
// Why this lives here and not in pyrfcx-core: pyrfc's errors carry the
// caller's *live Python object* and its *type* in `.args`
// (`('an integer required, received', True, 'of type', <class 'bool'>)`).
// pyrfcx-core is PyO3-free and never sees a Python object — by the time a
// value reaches the marshaller it is an `RfcValue`, and `IntEnum.FOO` has
// already become `Int(1)`, indistinguishable from a plain `1`. pyrfc's
// `fillVariable(container, name, TYPE, value)` knows the target RFCTYPE
// *and* the object at once; the only place we can reconstruct that pairing
// is here, against the type tree the core hands us (`describe_function`).
//
// This runs as a pre-pass before conversion and marshalling, and only when
// `compat=True`. The native path never sees it.
//
// Provenance (compat scope only): rules extracted from `_cyrfc.pyx`
// `fillVariable` / `fillStructureField` / `fillTable` / `fillString` in
// pyrfc 3.4 (https://github.com/SAP/PyRFC).

use std::ffi::{c_int, c_long, c_longlong};

use pyo3::PyTypeInfo;
use pyo3::exceptions::{PyOverflowError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyType};

use indexmap::IndexMap;
use pyrfcx_core::{FieldDesc, FuncDesc, RfcFieldType, RfcValue};

/// pyrfc config-bitmask flags that affect the input path (§B.3).
/// Both default to on, exactly as pyrfc's `config` defaults do.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CompatInputConfig {
    pub check_date: bool,
    pub check_time: bool,
}

impl Default for CompatInputConfig {
    fn default() -> Self {
        Self {
            check_date: true,
            check_time: true,
        }
    }
}

/// Whether validation should keep going or stand down.
///
/// `Defer` means we hit a name the descriptor does not know. pyrfc would
/// have failed there with the SDK's own "not found" error, raised from
/// `RfcGetParameterDescByName` / `RfcGetFieldDescByName` *before* filling
/// anything further. We cannot raise that error faithfully from here, so we
/// stop validating entirely and let `marshal::set_inputs` reach the same
/// name and produce the real SDK error. Stopping — rather than
/// skipping the one name and carrying on — is what preserves pyrfc's error
/// precedence: an unknown name must win over a type error that appears
/// later in the same call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Defer,
}

/// COMPAT-ONLY — NOT from the SDK; exists solely for pyrfc 3.4
/// retrocompatibility. A name as the SDK sees it under pyrfc: the text up
/// to the first NUL character.
///
/// pyrfc converts every name it hands the SDK (function, parameter, field
/// and `not_requested` names) to a NUL-terminated copy, so the SDK reads
/// the name only up to an embedded NUL: `RFCCHAR4\0junk` fills `RFCCHAR4`
/// and `NOPE\0RFCCHAR4` fails as `NOPE` (measured on real pyrfc). The facade
/// cuts each name the same way before the core sees it. Native pyrfcx
/// rejects such a name with `ValueError` instead.
pub(crate) fn pyrfc_sdk_name(name: &str) -> &str {
    pyrfc_c_text(name)
}

/// COMPAT-ONLY — NOT from the SDK; exists solely for pyrfc 3.4
/// retrocompatibility. The text up to the first NUL character: what the SDK
/// receives when pyrfc hands it a NUL-terminated copy (`fillString`) and
/// measures it with `strlenU`. One rule for names and text values.
fn pyrfc_c_text(text: &str) -> &str {
    match text.find('\0') {
        Some(end) => &text[..end],
        None => text,
    }
}

/// COMPAT-ONLY — NOT from the SDK; exists solely for pyrfc 3.4
/// retrocompatibility. Cuts every text value for a CHAR, STRING or UTCLONG
/// target at its first NUL, in parameters, structure fields and table rows.
///
/// pyrfc's `fillVariable` passes those values with a NUL-terminated length
/// (`strlenU`), so the SDK receives only the text before an embedded NUL:
/// `"A\0B"` arrives as `"A"` (measured on real pyrfc for CHAR and STRING).
/// Its other text targets need no cut: NUMC and the decimal family reject a
/// NUL in the client-side check first (on the uncut value), and DATE / TIME
/// go through `RfcSetDate` / `RfcSetTime` with their fixed width, NUL
/// included. Runs after the type-tree pre-pass, so that check sees the uncut
/// value as pyrfc's does. A name the descriptor does not know is left for
/// the SDK to reject. Native pyrfcx sends the whole value.
pub(crate) fn pyrfc_cut_text_values(inputs: &mut IndexMap<String, RfcValue>, desc: &FuncDesc) {
    for (name, value) in inputs.iter_mut() {
        if let Some(param) = desc.param(name) {
            cut_text_value(value, &param.desc);
        }
    }
}

fn cut_text_value(value: &mut RfcValue, desc: &FieldDesc) {
    match (value, desc.rfctype) {
        (
            RfcValue::String(text),
            RfcFieldType::Char | RfcFieldType::String | RfcFieldType::UtcLong,
        ) => {
            let end = pyrfc_c_text(text).len();
            text.truncate(end);
        }
        (RfcValue::Structure(fields), RfcFieldType::Structure) => cut_text_fields(fields, desc),
        (RfcValue::Table(rows), RfcFieldType::Table) => {
            for row in rows {
                cut_text_fields(row, desc);
            }
        }
        _ => {}
    }
}

fn cut_text_fields(fields: &mut IndexMap<String, RfcValue>, desc: &FieldDesc) {
    for (name, value) in fields.iter_mut() {
        if let Some(field) = desc.field(name) {
            cut_text_value(value, &field.desc);
        }
    }
}

/// Validate every caller-supplied parameter against the function's
/// signature, raising pyrfc 3.4's exact `TypeError`s and the `OverflowError`s
/// of its integer casts.
pub(crate) fn validate_inputs(
    py: Python<'_>,
    params: &Bound<'_, PyDict>,
    desc: &FuncDesc,
    cfg: CompatInputConfig,
) -> PyResult<()> {
    for (k, v) in params.iter() {
        let key: String = k.extract()?;
        let Some(param) = desc.param(pyrfc_sdk_name(&key)) else {
            return Ok(()); // Flow::Defer — see the enum's docs.
        };
        if validate_named(py, &v, &param.desc, &param.name, cfg)? == Flow::Defer {
            return Ok(());
        }
    }
    Ok(())
}

/// Validate one value and, on `TypeError`, append this field's canonical
/// name to `.args`.
///
/// pyrfc does this in `fillVariable`'s `except TypeError` arm — and because
/// that arm wraps the recursion into nested structures too, names accumulate
/// innermost-first: `(..., 'RFCINT1', 'IMPORTSTRUCT')`. The name appended is
/// the descriptor's (upper-case) spelling, not the caller's, because pyrfc
/// passes `paramDesc.name`/`fieldDesc.name` down to `fillVariable`.
/// Non-`TypeError` errors pass through untouched — pyrfc catches only
/// `TypeError`.
fn validate_named(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    desc: &FieldDesc,
    name: &str,
    cfg: CompatInputConfig,
) -> PyResult<Flow> {
    match validate_value(py, value, desc, cfg) {
        Err(err) => Err(append_field_name(py, err, name)),
        ok => ok,
    }
}

fn validate_value(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    desc: &FieldDesc,
    cfg: CompatInputConfig,
) -> PyResult<Flow> {
    match desc.rfctype {
        RfcFieldType::Structure => validate_structure(py, value, desc, cfg),
        RfcFieldType::Table => validate_table(py, value, desc, cfg),

        RfcFieldType::Char | RfcFieldType::String | RfcFieldType::UtcLong => {
            if is_exact_type::<PyString>(py, value) {
                Ok(Flow::Continue)
            } else {
                Err(type_error("an string is required, received", value))
            }
        }

        RfcFieldType::Numc => validate_numc(value),

        RfcFieldType::Bcd | RfcFieldType::Float | RfcFieldType::Decf16 | RfcFieldType::Decf34 => {
            validate_decimal(py, value)
        }

        RfcFieldType::Int | RfcFieldType::Int1 | RfcFieldType::Int2 => {
            validate_int(py, value, IntCast::CInt)
        }
        RfcFieldType::Int8 => validate_int(py, value, IntCast::CLongLong),

        RfcFieldType::Date => validate_date(py, value, cfg.check_date),
        RfcFieldType::Time => validate_time(py, value, cfg.check_time),

        RfcFieldType::Byte | RfcFieldType::XString => validate_bytes(value),

        // Types pyrfc's fillVariable does not handle at all: it raises
        // RFCError('Unknown RFC type %d when filling %s'). We cannot raise
        // pyrfc's RFCError from here without importing the facade (a layering
        // inversion), so validation stands down and the existing path decides.
        // Known, deliberate divergence. Unreachable for
        // every FM signature seen so far.
        _ => Ok(Flow::Continue),
    }
}

fn validate_structure(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    desc: &FieldDesc,
    cfg: CompatInputConfig,
) -> PyResult<Flow> {
    if !is_exact_type::<PyDict>(py, value) {
        return Err(type_error_type_only(
            "dictionary required for structure parameter, received",
            value,
        ));
    }
    let dict = value.cast::<PyDict>()?;
    for (k, v) in dict.iter() {
        let key: String = k.extract()?;
        let Some(field) = desc.field(pyrfc_sdk_name(&key)) else {
            return Ok(Flow::Defer);
        };
        if validate_named(py, &v, &field.desc, &field.name, cfg)? == Flow::Defer {
            return Ok(Flow::Defer);
        }
    }
    Ok(Flow::Continue)
}

fn validate_table(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    desc: &FieldDesc,
    cfg: CompatInputConfig,
) -> PyResult<Flow> {
    if !is_exact_type::<PyList>(py, value) {
        return Err(type_error_type_only(
            "list required for table parameter, received",
            value,
        ));
    }
    let list = value.cast::<PyList>()?;
    for row in list.iter() {
        // pyrfc's fillTable: a non-dict row is filled into the field named
        // '' (the single-unnamed-column convention, §B.4) — an SDK-level
        // outcome, not a client-side type check. Stand down for those.
        if !is_exact_type::<PyDict>(py, &row) {
            return Ok(Flow::Defer);
        }
        let dict = row.cast::<PyDict>()?;
        for (k, v) in dict.iter() {
            let key: String = k.extract()?;
            let Some(field) = desc.field(pyrfc_sdk_name(&key)) else {
                return Ok(Flow::Defer);
            };
            if validate_named(py, &v, &field.desc, &field.name, cfg)? == Flow::Defer {
                return Ok(Flow::Defer);
            }
        }
    }
    Ok(Flow::Continue)
}

/// The C integer type pyrfc's setter declares: `RfcSetInt` takes a C `int`
/// (INT/INT1/INT2), `RfcSetInt8` a C `long long` (INT8).
#[derive(Debug, Clone, Copy)]
enum IntCast {
    CInt,
    CLongLong,
}

/// INT/INT1/INT2/INT8: `type(value) is int`, then Cython converts the value
/// to the setter's C type at the call. An out-of-range value raises
/// CPython's own `OverflowError` — not a `TypeError`, so no field name is
/// appended. We perform the same conversion so CPython writes the message.
/// A value that fits reaches the SDK: INT1/INT2 then wrap silently.
fn validate_int(py: Python<'_>, value: &Bound<'_, PyAny>, cast: IntCast) -> PyResult<Flow> {
    if !is_exact_type::<PyInt>(py, value) {
        return Err(type_error("an integer required, received", value));
    }
    match cast {
        IntCast::CInt => {
            // Cython goes through `PyLong_AsLong` when a C int is no wider
            // than a C long, then narrows. Where the two widths are equal
            // (Windows) the narrowing never fails. Where it fails, Cython
            // names the setter's parameter type, `RFC_INT`.
            let long: c_long = value.extract()?;
            if c_int::try_from(long).is_err() {
                return Err(PyOverflowError::new_err(
                    "value too large to convert to RFC_INT",
                ));
            }
        }
        IntCast::CLongLong => {
            value.extract::<c_longlong>()?;
        }
    }
    Ok(Flow::Continue)
}

/// BYTE/XSTRING (`fillBytes`): `len(value)` first — CPython's own
/// `TypeError` for an int or None — then a Cython `<char*>` cast, which
/// accepts `bytes` and `bytearray` (subclasses too) and raises
/// "expected bytes, <tp_name> found" for anything else.
fn validate_bytes(value: &Bound<'_, PyAny>) -> PyResult<Flow> {
    value.len()?;
    if value.is_instance_of::<PyBytes>() || value.is_instance_of::<PyByteArray>() {
        return Ok(Flow::Continue);
    }
    Err(PyTypeError::new_err(format!(
        "expected bytes, {} found",
        tp_name(&value.get_type())?
    )))
}

/// CPython's `tp_name`, which is what its own messages print. The stable
/// ABI cannot read it, so it is rebuilt from the flags: a Python class (a
/// mutable heap type) and a builtin print `__name__`; a type defined in an
/// extension module, static or created from a spec, prints
/// `module.__name__` (`datetime.date`, `collections.deque`).
fn tp_name(ty: &Bound<'_, PyType>) -> PyResult<String> {
    const PY_TPFLAGS_IMMUTABLETYPE: u64 = 1 << 8;
    const PY_TPFLAGS_HEAPTYPE: u64 = 1 << 9;
    let name = ty.name()?.to_string();
    let module = ty.module()?.to_string();
    let flags: u64 = ty.getattr("__flags__")?.extract()?;
    let python_class = flags & PY_TPFLAGS_HEAPTYPE != 0 && flags & PY_TPFLAGS_IMMUTABLETYPE == 0;
    if python_class || module == "builtins" {
        Ok(name)
    } else {
        Ok(format!("{module}.{name}"))
    }
}

/// NUMC: pyrfc requires `value.isdigit()` to be truthy, then hands the value
/// to `fillString`, which needs `.encode()` — so `b'123'` passes `isdigit()`
/// but dies in `fillString`. Both failures land in the same `except`.
fn validate_numc(value: &Bound<'_, PyAny>) -> PyResult<Flow> {
    let is_digit = value
        .call_method0("isdigit")
        .and_then(|r| r.is_truthy())
        .unwrap_or(false);
    let fillable = value.hasattr("encode").unwrap_or(false);

    if is_digit && fillable {
        Ok(Flow::Continue)
    } else {
        Err(type_error("a numeric string is required, received", value))
    }
}

/// BCD / FLOAT / DECF16 / DECF34: pyrfc stringifies exact `float`/`Decimal`,
/// passes anything else through, requires `Decimal(svalue)` to construct,
/// then requires `fillString(svalue)` to work (i.e. `.encode()` — which is
/// what rejects a plain `int`, even though `Decimal(5)` is perfectly valid).
fn validate_decimal(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Flow> {
    let decimal_cls = py.import("decimal")?.getattr("Decimal")?;

    let stringify = is_exact_type::<PyFloat>(py, value) || value.get_type().is(&decimal_cls);
    let svalue = if stringify {
        value.str()?.into_any()
    } else {
        value.clone()
    };

    // NOTE: pyrfc substitutes a non-"." locale radix here (via its
    // process-global `_LOCALE_RADIX`, settable with `set_locale_radix`).
    // The facade exposes neither, so we mirror the "." branch only —
    // tracked with the deferred `config` work.
    let parses = decimal_cls.call1((&svalue,)).is_ok();
    let fillable = svalue.hasattr("encode").unwrap_or(false);

    if parses && fillable {
        Ok(Flow::Continue)
    } else {
        Err(type_error("a decimal value required, received", value))
    }
}

/// DATE (§B.3): falsy is skipped silently; exact `datetime.date` passes (a
/// `datetime.datetime` does NOT — it is a subclass); `str` is validated only
/// when `check_date` is on. The literal initial date `'00000000'` is
/// REJECTED, because `date(0, 0, 0)` raises — pyrfc's sharp edge, kept.
fn validate_date(py: Python<'_>, value: &Bound<'_, PyAny>, check_date: bool) -> PyResult<Flow> {
    if !value.is_truthy()? {
        return Ok(Flow::Continue); // falsy → setter never called, no error
    }

    let date_cls = py.import("datetime")?.getattr("date")?;
    if value.get_type().is(&date_cls) {
        return Ok(Flow::Continue);
    }
    if !is_exact_type::<PyString>(py, value) {
        return Err(type_error("date value required, received", value));
    }
    if !check_date {
        return Ok(Flow::Continue);
    }

    let s: String = value.extract()?;
    let chars: Vec<char> = s.chars().collect();
    if chars.len() != 8 {
        return Err(type_error("date value required, received", value));
    }
    // pyrfc uses .strip() here (both ends) — asymmetric with TIME's .rstrip().
    if s.trim().is_empty() {
        return Ok(Flow::Continue);
    }

    let ok = (|| -> PyResult<()> {
        let y = py_int(py, &chars[0..4])?;
        let m = py_int(py, &chars[4..6])?;
        let d = py_int(py, &chars[6..8])?;
        date_cls.call1((y, m, d))?;
        Ok(())
    })()
    .is_ok();

    if ok {
        Ok(Flow::Continue)
    } else {
        Err(type_error("date value required, received", value))
    }
}

/// TIME (§B.3): as DATE, but blank-detection uses `.rstrip()` (trailing
/// only) and `'000000'` is valid midnight, so it passes.
fn validate_time(py: Python<'_>, value: &Bound<'_, PyAny>, check_time: bool) -> PyResult<Flow> {
    if !value.is_truthy()? {
        return Ok(Flow::Continue);
    }

    let time_cls = py.import("datetime")?.getattr("time")?;
    if value.get_type().is(&time_cls) {
        return Ok(Flow::Continue);
    }
    if !is_exact_type::<PyString>(py, value) {
        return Err(type_error("time value required, received", value));
    }
    if !check_time {
        return Ok(Flow::Continue);
    }

    let s: String = value.extract()?;
    let chars: Vec<char> = s.chars().collect();
    if chars.len() != 6 {
        return Err(type_error("time value required, received", value));
    }
    if s.trim_end().is_empty() {
        return Ok(Flow::Continue);
    }

    let ok = (|| -> PyResult<()> {
        let h = py_int(py, &chars[0..2])?;
        let m = py_int(py, &chars[2..4])?;
        let sec = py_int(py, &chars[4..6])?;
        time_cls.call1((h, m, sec))?;
        Ok(())
    })()
    .is_ok();

    if ok {
        Ok(Flow::Continue)
    } else {
        Err(type_error("time value required, received", value))
    }
}

// ── helpers ────────────────────────────────────────────────────────────

/// `type(value) is T` — an exact-type check, never `isinstance`. This is
/// what makes pyrfc reject `bool` and `IntEnum` for INT targets, and
/// `datetime.datetime` for DATE targets.
fn is_exact_type<T: PyTypeInfo>(py: Python<'_>, value: &Bound<'_, PyAny>) -> bool {
    value.get_type().is(T::type_object(py))
}

/// Python's `int(s)` on a slice of characters — not Rust's `parse`, because
/// pyrfc calls the builtin, which accepts surrounding whitespace and a
/// leading sign.
fn py_int<'py>(py: Python<'py>, chars: &[char]) -> PyResult<Bound<'py, PyAny>> {
    let s: String = chars.iter().collect();
    PyInt::type_object(py).call1((s,))
}

/// `TypeError(msg, value, 'of type', type(value))` — the four-argument
/// template used by every scalar arm of `fillVariable`.
fn type_error(msg: &str, value: &Bound<'_, PyAny>) -> PyErr {
    PyTypeError::new_err((
        msg.to_string(),
        value.clone().unbind(),
        "of type".to_string(),
        value.get_type().unbind(),
    ))
}

/// `TypeError(msg, str(type(value)))` — the two-argument template used by
/// the STRUCTURE and TABLE arms. Note it carries `str(type(value))` (a
/// string) and *not* the value itself; that asymmetry is pyrfc's, kept.
fn type_error_type_only(msg: &str, value: &Bound<'_, PyAny>) -> PyErr {
    let ty_repr = match value.get_type().str() {
        Ok(s) => s.to_string(),
        Err(e) => return e,
    };
    PyTypeError::new_err((msg.to_string(), ty_repr))
}

/// `e.args += (name,)`, for `TypeError` only.
fn append_field_name(py: Python<'_>, err: PyErr, name: &str) -> PyErr {
    if !err.is_instance_of::<PyTypeError>(py) {
        return err;
    }
    {
        let value = err.value(py);
        if let Ok(args) = value.getattr("args")
            && let Ok(extended) = args.add((name,))
        {
            let _ = value.setattr("args", extended);
        }
    }
    err
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyrfcx_core::{NamedFieldDesc, ParamDesc, RfcDirection};

    fn scalar(rfctype: RfcFieldType, uc_length: u32) -> FieldDesc {
        FieldDesc {
            rfctype,
            nuc_length: uc_length,
            uc_length,
            decimals: 0,
            type_info: None,
            fields: Vec::new(),
        }
    }

    /// STFC_STRUCTURE-shaped: IMPORTSTRUCT (structure) + RFCTABLE (table of
    /// the same line type), both carrying RFCINT1/RFCCHAR4/RFCDATE/RFCTIME.
    fn stfc_structure() -> FuncDesc {
        let line = vec![
            NamedFieldDesc {
                name: "RFCINT1".into(),
                nuc_offset: 0,
                uc_offset: 0,
                desc: scalar(RfcFieldType::Int1, 1),
            },
            NamedFieldDesc {
                name: "RFCCHAR4".into(),
                nuc_offset: 0,
                uc_offset: 0,
                desc: scalar(RfcFieldType::Char, 8),
            },
            NamedFieldDesc {
                name: "RFCDATE".into(),
                nuc_offset: 0,
                uc_offset: 0,
                desc: scalar(RfcFieldType::Date, 16),
            },
            NamedFieldDesc {
                name: "RFCTIME".into(),
                nuc_offset: 0,
                uc_offset: 0,
                desc: scalar(RfcFieldType::Time, 12),
            },
            NamedFieldDesc {
                name: "RFCFLOAT".into(),
                nuc_offset: 0,
                uc_offset: 0,
                desc: scalar(RfcFieldType::Float, 8),
            },
            NamedFieldDesc {
                name: "RFCINT4".into(),
                nuc_offset: 0,
                uc_offset: 0,
                desc: scalar(RfcFieldType::Int, 4),
            },
            NamedFieldDesc {
                name: "RFCHEX3".into(),
                nuc_offset: 0,
                uc_offset: 0,
                desc: scalar(RfcFieldType::Byte, 3),
            },
            // Not in STFC_STRUCTURE: a synthetic INT8 field for the cast test.
            NamedFieldDesc {
                name: "CV_INT8".into(),
                nuc_offset: 0,
                uc_offset: 0,
                desc: scalar(RfcFieldType::Int8, 8),
            },
        ];
        FuncDesc {
            name: "STFC_STRUCTURE".into(),
            params: vec![
                ParamDesc {
                    name: "IMPORTSTRUCT".into(),
                    direction: RfcDirection::Import,
                    default_value: String::new(),
                    text: String::new(),
                    optional: false,
                    desc: FieldDesc {
                        rfctype: RfcFieldType::Structure,
                        nuc_length: 0,
                        uc_length: 0,
                        decimals: 0,
                        type_info: None,
                        fields: line.clone(),
                    },
                },
                ParamDesc {
                    name: "RFCTABLE".into(),
                    direction: RfcDirection::Tables,
                    default_value: String::new(),
                    text: String::new(),
                    optional: false,
                    desc: FieldDesc {
                        rfctype: RfcFieldType::Table,
                        nuc_length: 0,
                        uc_length: 0,
                        decimals: 0,
                        type_info: None,
                        fields: line,
                    },
                },
            ],
        }
    }

    /// Run the pre-pass over a params dict built from Python source.
    fn check(py_src: &str) -> Result<(), Vec<String>> {
        Python::initialize();
        Python::attach(|py| {
            let params = py
                .eval(&std::ffi::CString::new(py_src).unwrap(), None, None)
                .expect("params source must evaluate")
                .cast_into::<PyDict>()
                .expect("params source must be a dict");
            match validate_inputs(py, &params, &stfc_structure(), CompatInputConfig::default()) {
                Ok(()) => Ok(()),
                Err(e) => {
                    let args = e.value(py).getattr("args").unwrap();
                    // Render each arg with repr() so the assertions below can
                    // pin types (`<class 'bool'>`) as well as values.
                    let rendered = args
                        .try_iter()
                        .unwrap()
                        .map(|a| a.unwrap().repr().unwrap().to_string())
                        .collect();
                    Err(rendered)
                }
            }
        })
    }

    #[test]
    fn int_target_takes_int_only_and_reports_the_exact_type() {
        // `type(value) is int` — an exact check, so bool and IntEnum are out
        // even though both are ints by isinstance. This pairing of target
        // type with live Python object is the whole reason this pre-pass lives here.
        let err = check("{'IMPORTSTRUCT': {'RFCINT1': True}}").unwrap_err();
        assert_eq!(
            err,
            vec![
                "'an integer required, received'",
                "True",
                "'of type'",
                "<class 'bool'>",
                "'RFCINT1'",
                "'IMPORTSTRUCT'",
            ]
        );

        let err = check("{'IMPORTSTRUCT': {'RFCINT1': '1'}}").unwrap_err();
        assert_eq!(err[3], "<class 'str'>");

        assert!(check("{'IMPORTSTRUCT': {'RFCINT1': 1}}").is_ok());
    }

    #[test]
    fn int_target_rejects_int_subclasses() {
        let err =
            check("{'IMPORTSTRUCT': {'RFCINT1': __import__('enum').IntEnum('E', 'ONE').ONE}}")
                .unwrap_err();
        assert_eq!(err[0], "'an integer required, received'");
        assert_eq!(err[3], "<enum 'E'>");
    }

    #[test]
    fn field_names_accumulate_innermost_first() {
        // pyrfc appends one name per nesting level, in reverse: the field
        // first, then the parameter that contains it.
        let err = check("{'IMPORTSTRUCT': {'RFCCHAR4': 1}}").unwrap_err();
        assert_eq!(err[4], "'RFCCHAR4'");
        assert_eq!(err[5], "'IMPORTSTRUCT'");

        // Through a table the container name differs but the order holds.
        let err = check("{'RFCTABLE': [{'RFCCHAR4': 1}]}").unwrap_err();
        assert_eq!(err[4], "'RFCCHAR4'");
        assert_eq!(err[5], "'RFCTABLE'");
    }

    #[test]
    fn container_mismatch_uses_the_two_arg_template() {
        // STRUCTURE/TABLE carry str(type(value)) and NOT the value — the
        // asymmetry is pyrfc's.
        let err = check("{'IMPORTSTRUCT': [{'RFCINT1': 1}]}").unwrap_err();
        assert_eq!(
            err,
            vec![
                "'dictionary required for structure parameter, received'",
                "\"<class 'list'>\"",
                "'IMPORTSTRUCT'",
            ]
        );

        let err = check("{'RFCTABLE': {'RFCINT1': 1}}").unwrap_err();
        assert_eq!(err[0], "'list required for table parameter, received'");
        assert_eq!(err[1], "\"<class 'dict'>\"");
        assert_eq!(err[2], "'RFCTABLE'");
    }

    #[test]
    fn date_validation_including_the_initial_date_sharp_edge() {
        assert!(check("{'IMPORTSTRUCT': {'RFCDATE': '20161231'}}").is_ok());
        // Falsy → the setter is never called; not an error.
        assert!(check("{'IMPORTSTRUCT': {'RFCDATE': ''}}").is_ok());
        // 8 blanks → strip() empties it → validation skipped.
        assert!(check("{'IMPORTSTRUCT': {'RFCDATE': '        '}}").is_ok());
        assert!(
            check("{'IMPORTSTRUCT': {'RFCDATE': __import__('datetime').date(2018, 6, 25)}}")
                .is_ok()
        );

        // The literal initial date is REJECTED: date(0, 0, 0) raises. Only
        // ''/None (skip) or blanks express "initial" — pyrfc's sharp edge.
        let err = check("{'IMPORTSTRUCT': {'RFCDATE': '00000000'}}").unwrap_err();
        assert_eq!(err[0], "'date value required, received'");

        // datetime is a date subclass, so the exact-type check rejects it.
        let err =
            check("{'IMPORTSTRUCT': {'RFCDATE': __import__('datetime').datetime(2018, 6, 25)}}")
                .unwrap_err();
        assert_eq!(err[3], "<class 'datetime.datetime'>");

        for bad in ["'2016123'", "'201612311'", "'20161232'", "20161231"] {
            assert!(
                check(&format!("{{'IMPORTSTRUCT': {{'RFCDATE': {bad}}}}}")).is_err(),
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn time_validation_is_asymmetric_with_date() {
        // '000000' is midnight, not a sentinel — it passes.
        assert!(check("{'IMPORTSTRUCT': {'RFCTIME': '000000'}}").is_ok());
        assert!(check("{'IMPORTSTRUCT': {'RFCTIME': '123456'}}").is_ok());
        // TIME blank-detects with rstrip() (trailing only), unlike DATE's strip().
        assert!(check("{'IMPORTSTRUCT': {'RFCTIME': '      '}}").is_ok());

        for bad in ["'12345'", "'1234566'", "'123466'", "123456"] {
            assert!(
                check(&format!("{{'IMPORTSTRUCT': {{'RFCTIME': {bad}}}}}")).is_err(),
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn decimal_target_parses_but_rejects_plain_int() {
        assert!(check("{'IMPORTSTRUCT': {'RFCFLOAT': 1.2}}").is_ok());
        assert!(check("{'IMPORTSTRUCT': {'RFCFLOAT': '1.2'}}").is_ok());
        assert!(
            check("{'IMPORTSTRUCT': {'RFCFLOAT': __import__('decimal').Decimal('1.2')}}").is_ok()
        );

        // Decimal(5) is perfectly valid, but pyrfc then calls fillString(5),
        // which needs .encode() — so a plain int is rejected all the same.
        let err = check("{'IMPORTSTRUCT': {'RFCFLOAT': 5}}").unwrap_err();
        assert_eq!(err[0], "'a decimal value required, received'");
        assert_eq!(err[3], "<class 'int'>");

        // Non-numeric strings: the SDK would silently garbage-convert these
        // on a BCD target, so this check is load-bearing (probed).
        assert!(check("{'IMPORTSTRUCT': {'RFCFLOAT': 'A'}}").is_err());
        assert!(check("{'IMPORTSTRUCT': {'RFCFLOAT': '1,2'}}").is_err());
    }

    #[test]
    fn int_beyond_the_setter_c_type_raises_overflow_without_a_field_name() {
        // OverflowError is not a TypeError, so pyrfc appends no field name.
        // The message depends on the width of a C long. Where it is as wide
        // as a C int (Windows), the C long conversion fails for INT and the
        // long long conversion for INT8. Where it is 64 bits wide, the C long
        // conversion succeeds for INT and the narrowing to int fails, and
        // INT8 goes through the C long conversion.
        let long_is_int = std::mem::size_of::<c_long>() == std::mem::size_of::<c_int>();
        let (int4_message, int8_message) = if long_is_int {
            (
                "'Python int too large to convert to C long'",
                "'int too big to convert'",
            )
        } else {
            (
                "'value too large to convert to RFC_INT'",
                "'Python int too large to convert to C long'",
            )
        };
        for v in ["2**31", "-2**31 - 1"] {
            let err = check(&format!("{{'IMPORTSTRUCT': {{'RFCINT4': {v}}}}}")).unwrap_err();
            assert_eq!(err, vec![int4_message]);
        }
        for v in ["2**63", "-2**63 - 1"] {
            let err = check(&format!("{{'IMPORTSTRUCT': {{'CV_INT8': {v}}}}}")).unwrap_err();
            assert_eq!(err, vec![int8_message]);
        }
        // Within a C int, INT1 reaches the SDK, which wraps it.
        assert!(check("{'IMPORTSTRUCT': {'RFCINT1': 300}}").is_ok());
        assert!(check("{'IMPORTSTRUCT': {'CV_INT8': 2**63 - 1}}").is_ok());
    }

    #[test]
    fn byte_target_mirrors_fill_bytes() {
        let err = check("{'IMPORTSTRUCT': {'RFCHEX3': 'abc'}}").unwrap_err();
        assert_eq!(
            err,
            vec!["'expected bytes, str found'", "'RFCHEX3'", "'IMPORTSTRUCT'"]
        );
        let err = check("{'RFCTABLE': [{'RFCHEX3': 5}]}").unwrap_err();
        assert_eq!(
            err,
            vec![
                "\"object of type 'int' has no len()\"",
                "'RFCHEX3'",
                "'RFCTABLE'"
            ]
        );
        // tp_name, not __name__, for a type from an extension module.
        let err =
            check("{'IMPORTSTRUCT': {'RFCHEX3': __import__('collections').deque()}}").unwrap_err();
        assert_eq!(err[0], "'expected bytes, collections.deque found'");
        let err = check(&format!(
            "{{'IMPORTSTRUCT': {{'RFCHEX3': {}}}}}",
            "type('Mine', (), {'__len__': lambda s: 0})()"
        ))
        .unwrap_err();
        assert_eq!(err[0], "'expected bytes, Mine found'");
        assert!(check("{'IMPORTSTRUCT': {'RFCHEX3': b'abc'}}").is_ok());
        assert!(check("{'IMPORTSTRUCT': {'RFCHEX3': bytearray(b'abc')}}").is_ok());
    }

    #[test]
    fn check_date_and_check_time_off_bypass_validation() {
        Python::initialize();
        Python::attach(|py| {
            let cfg = CompatInputConfig {
                check_date: false,
                check_time: false,
            };
            let src = std::ffi::CString::new(
                "{'IMPORTSTRUCT': {'RFCDATE': '00000000', 'RFCTIME': '999999'}}",
            )
            .unwrap();
            let params = py
                .eval(&src, None, None)
                .unwrap()
                .cast_into::<PyDict>()
                .unwrap();
            assert!(validate_inputs(py, &params, &stfc_structure(), cfg).is_ok());

            // The type check is not part of check_date — it always applies.
            let src = std::ffi::CString::new("{'IMPORTSTRUCT': {'RFCDATE': 20161231}}").unwrap();
            let params = py
                .eval(&src, None, None)
                .unwrap()
                .cast_into::<PyDict>()
                .unwrap();
            assert!(validate_inputs(py, &params, &stfc_structure(), cfg).is_err());
        });
    }

    #[test]
    fn text_values_for_char_targets_are_cut_at_the_first_nul() {
        let text = |s: &str| RfcValue::String(s.into());
        let line = |c: &str, d: &str| {
            IndexMap::from([
                ("RFCCHAR4".to_owned(), text(c)),
                ("RFCDATE".to_owned(), text(d)),
            ])
        };
        let mut inputs = IndexMap::from([
            (
                "IMPORTSTRUCT".to_owned(),
                RfcValue::Structure(line("A\0B", "2026\x00101")),
            ),
            (
                "RFCTABLE".to_owned(),
                RfcValue::Table(vec![line("\0", "2026\x00101")]),
            ),
            ("NOSUCHPARAM".to_owned(), text("A\0B")),
        ]);
        pyrfc_cut_text_values(&mut inputs, &stfc_structure());
        // CHAR is cut; DATE keeps its NUL (pyrfc's RfcSetDate takes the
        // fixed width); an unknown name is left for the SDK.
        let expected = IndexMap::from([
            (
                "IMPORTSTRUCT".to_owned(),
                RfcValue::Structure(line("A", "2026\x00101")),
            ),
            (
                "RFCTABLE".to_owned(),
                RfcValue::Table(vec![line("", "2026\x00101")]),
            ),
            ("NOSUCHPARAM".to_owned(), text("A\0B")),
        ]);
        assert_eq!(inputs, expected);
    }

    #[test]
    fn unknown_names_stand_down_to_the_marshaller() {
        // An unknown parameter must not be validated here — the SDK reports
        // it. Crucially the pre-pass stops entirely, so a type error later in
        // the same call cannot jump ahead of the SDK's "not found" error.
        assert!(check("{'NOSUCHPARAM': 1, 'IMPORTSTRUCT': {'RFCINT1': True}}").is_ok());
        // Same for an unknown field inside a known structure.
        assert!(check("{'IMPORTSTRUCT': {'NOSUCHFIELD': 1}, 'RFCTABLE': 'not a list'}").is_ok());
    }
}
