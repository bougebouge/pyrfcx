// Metadata-driven marshalling: set_inputs (Python→SDK) and read_outputs (SDK→Python).

use indexmap::IndexMap;

use crate::connection::CompatOptions;
use crate::error::{RfcError, RfcErrorInfo};
use crate::ffi::{
    self, DATA_CONTAINER_HANDLE, RFC_DIRECTION, RFC_FIELD_DESC, RFC_FUNCTION_DESC_HANDLE,
    RFC_FUNCTION_HANDLE, RFC_PARAMETER_DESC, RFC_RC, RFC_STRUCTURE_HANDLE, RFC_TABLE_HANDLE,
    RFC_TYPE_DESC_HANDLE, RFCTYPE,
};
use crate::uc;
use crate::value::RfcValue;

// ── FieldLike trait ────────────────────────────────────────────────────
// Abstracts over RFC_PARAMETER_DESC and RFC_FIELD_DESC — both carry
// type_, ucLength, decimals, typeDescHandle.

trait FieldLike {
    fn rfctype(&self) -> RFCTYPE;
    fn uc_length(&self) -> u32;
    #[allow(dead_code)] // used by the BCD round-trip tests
    fn decimals(&self) -> u32;
    fn type_desc_handle(&self) -> RFC_TYPE_DESC_HANDLE;
}

impl FieldLike for RFC_PARAMETER_DESC {
    fn rfctype(&self) -> RFCTYPE {
        self.type_
    }
    fn uc_length(&self) -> u32 {
        self.ucLength
    }
    fn decimals(&self) -> u32 {
        self.decimals
    }
    fn type_desc_handle(&self) -> RFC_TYPE_DESC_HANDLE {
        self.typeDescHandle
    }
}

impl FieldLike for RFC_FIELD_DESC {
    fn rfctype(&self) -> RFCTYPE {
        self.type_
    }
    fn uc_length(&self) -> u32 {
        self.ucLength
    }
    fn decimals(&self) -> u32 {
        self.decimals
    }
    fn type_desc_handle(&self) -> RFC_TYPE_DESC_HANDLE {
        self.typeDescHandle
    }
}

// ── compat input predicates ───────────────────────────────────────────

/// In compat mode, empty-string inputs targeting DATE or TIME
/// fields are silently skipped (no setter call), matching pyrfc 3.4.
pub(crate) fn skip_input(value: &RfcValue, rfctype: RFCTYPE, compat: CompatOptions) -> bool {
    if !compat.skip_empty_date_time_inputs {
        return false;
    }
    if let RfcValue::String(s) = value {
        s.is_empty() && (rfctype == RFCTYPE::RFCTYPE_DATE || rfctype == RFCTYPE::RFCTYPE_TIME)
    } else {
        false
    }
}

/// Check that a string/char value fits the target field's width.
/// Returns `Ok(())` if the value fits or the target is not fixed-length.
/// Returns `Err(InvalidInput)` if over-long, unless the compat flag is set.
pub(crate) fn check_input_length(
    value: &RfcValue,
    rfctype: RFCTYPE,
    uc_length: u32,
    path: &str,
    compat: CompatOptions,
) -> Result<(), RfcError> {
    if compat.skip_input_length_check {
        return Ok(());
    }
    match rfctype {
        RFCTYPE::RFCTYPE_CHAR | RFCTYPE::RFCTYPE_NUM => {
            let owned;
            let s = match value {
                RfcValue::Char(s)
                | RfcValue::String(s)
                | RfcValue::Numc(s)
                | RfcValue::Decimal(s)
                | RfcValue::Date(s)
                | RfcValue::Time(s)
                | RfcValue::Temporal(s) => s.as_str(),
                RfcValue::Float(f) => {
                    owned = f.to_string();
                    owned.as_str()
                }
                // Integer values are length-checked against NUMC
                // only — for CHAR they are rejected as a type mismatch
                // in set_field, where a width error would mislead.
                RfcValue::Int(_)
                | RfcValue::Int1(_)
                | RfcValue::Int2(_)
                | RfcValue::Int4(_)
                | RfcValue::Int8(_)
                    if rfctype == RFCTYPE::RFCTYPE_NUM =>
                {
                    owned = int_value(value)
                        .expect("guard matched an integer variant")
                        .to_string();
                    owned.as_str()
                }
                _ => return Ok(()),
            };
            let field_width = (uc_length / 2) as usize;
            let encoded_len: usize = s.encode_utf16().count();
            if encoded_len > field_width {
                let type_name = if rfctype == RFCTYPE::RFCTYPE_CHAR {
                    "CHAR"
                } else {
                    "NUMC"
                };
                return Err(RfcError::InvalidInput(format!(
                    "input for {path} exceeds the field width: \
                     {encoded_len} UTF-16 units, field is {type_name}({field_width})"
                )));
            }
            Ok(())
        }
        RFCTYPE::RFCTYPE_BYTE => {
            if let RfcValue::Bytes(b) = value {
                let field_len = uc_length as usize;
                if b.len() > field_len {
                    return Err(RfcError::InvalidInput(format!(
                        "input for {path} exceeds the field width: \
                         {} bytes, field is BYTE({field_len})",
                        b.len()
                    )));
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Is `s` a syntactically valid decimal number for a BCD/DECF field?
///
/// The SDK's `RfcSetString` on a packed-decimal (BCD) target **silently
/// garbage-converts** non-numeric input — `"A"` → `1.00`, `"x9y"` → `899.00`
/// (probed, 750 PL 18) — rather than erroring like FLOAT/DECF do. The native
/// marshaller rejects such input before the setter; this predicate is the
/// gate. It mirrors Python's `Decimal` grammar (pyrfc's own validity notion,
/// so native and compat agree on what "numeric" means): optional surrounding
/// ASCII whitespace, an optional sign, then either a finite number
/// (`digits`, optional `.`, optional fractional `digits`, optional
/// `[eE][+-]?digits` exponent) or a special value (`inf`/`infinity`/`nan`/
/// `snan`, case-insensitive, `nan` optionally trailed by digits). An empty
/// string is not a number (`Decimal("")` raises).
fn is_valid_decimal(s: &str) -> bool {
    let s = s.trim_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    if body.is_empty() {
        return false;
    }

    // Special values: Infinity / NaN (Python Decimal accepts these).
    let lower = body.to_ascii_lowercase();
    if lower == "inf" || lower == "infinity" {
        return true;
    }
    if let Some(rest) = lower
        .strip_prefix("snan")
        .or_else(|| lower.strip_prefix("nan"))
    {
        return rest.chars().all(|c| c.is_ascii_digit());
    }

    // Finite number: mantissa [ exponent ]
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e)),
        None => (body, None),
    };

    // Mantissa: digits ['.' digits] | ['.'] digits — at least one digit.
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((i, f)) => (i, f),
        None => (mantissa, ""),
    };
    let digits_ok = |p: &str| p.chars().all(|c| c.is_ascii_digit());
    if !digits_ok(int_part) || !digits_ok(frac_part) {
        return false;
    }
    if int_part.is_empty() && frac_part.is_empty() {
        return false; // "." or "" — no digit anywhere
    }

    // Exponent: [sign] digits, at least one digit.
    if let Some(e) = exponent {
        let e_body = e.strip_prefix(['+', '-']).unwrap_or(e);
        if e_body.is_empty() || !e_body.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    true
}

/// The i128 value of an integer-carrying variant, `None` otherwise.
fn int_value(v: &RfcValue) -> Option<i128> {
    match v {
        RfcValue::Int(n) => Some(*n),
        RfcValue::Int1(n) => Some(i128::from(*n)),
        RfcValue::Int2(n) => Some(i128::from(*n)),
        RfcValue::Int4(n) => Some(i128::from(*n)),
        RfcValue::Int8(n) => Some(i128::from(*n)),
        _ => None,
    }
}

// ── set_inputs ─────────────────────────────────────────────────────────

/// Populate IMPORT/CHANGING/TABLES parameters on a function handle from
/// the caller-supplied map. Unknown parameter names are rejected.
pub(crate) fn set_inputs(
    sdk: &ffi::SapNwRfc,
    func_desc: RFC_FUNCTION_DESC_HANDLE,
    func_handle: RFC_FUNCTION_HANDLE,
    inputs: &IndexMap<String, RfcValue>,
    compat: CompatOptions,
) -> Result<(), RfcError> {
    for (name, value) in inputs {
        let name_uc = uc::encode_nul_terminated(name, || "parameter name".into())?;

        let mut param_desc: RFC_PARAMETER_DESC = unsafe { std::mem::zeroed() };
        let mut error_info = ffi::zero_error_info();

        // Safety: func_desc is a valid function descriptor handle from
        // the SDK cache. name_uc is a valid null-terminated UTF-16 string.
        let rc = unsafe {
            sdk.RfcGetParameterDescByName(
                func_desc,
                name_uc.as_ptr(),
                &mut param_desc,
                &mut error_info,
            )
        };
        if rc != RFC_RC::RFC_OK {
            let info = ffi::extract_error_info("RfcGetParameterDescByName", &error_info);
            return Err(RfcError::from_error_info(info));
        }

        // Reject EXPORT-only parameters (InvalidInput: a wrong value, not
        // an SDK failure).
        if param_desc.direction == RFC_DIRECTION::RFC_EXPORT {
            return Err(RfcError::InvalidInput(format!(
                "parameter {name} is EXPORT-only and cannot be set"
            )));
        }

        if skip_input(value, param_desc.type_, compat) {
            continue;
        }

        set_field(sdk, func_handle, &name_uc, value, &param_desc, compat, name)?;
    }
    Ok(())
}

// ── read_outputs ───────────────────────────────────────────────────────

/// Read all EXPORT/CHANGING/TABLES parameters from a function handle
/// after `RfcInvoke`, returning an ordered map in descriptor order.
pub(crate) fn read_outputs(
    sdk: &ffi::SapNwRfc,
    func_desc: RFC_FUNCTION_DESC_HANDLE,
    func_handle: RFC_FUNCTION_HANDLE,
    compat: CompatOptions,
) -> Result<IndexMap<String, RfcValue>, RfcError> {
    let param_count = get_parameter_count(sdk, func_desc)?;
    let mut result = IndexMap::with_capacity(param_count as usize);
    for_each_output(
        sdk,
        func_desc,
        func_handle,
        compat,
        |name, name_uc, param_desc| {
            let value = read_field(sdk, func_handle, name_uc, param_desc)?;
            result.insert(name, value);
            Ok(())
        },
    )?;
    Ok(result)
}

/// One output parameter of a function container, as `open_outputs` hands
/// it over: a fully read value, or an unread top-level table.
pub(crate) enum OutputSlot {
    Value(RfcValue),
    Table {
        handle: RFC_TABLE_HANDLE,
        type_desc: RFC_TYPE_DESC_HANDLE,
        rows: u32,
    },
}

/// Like `read_outputs` — same parameters, same order, same inclusion
/// rules — but a top-level TABLE parameter is opened and left unread, so
/// its rows can be taken in chunks (`read_rows` + `delete_last_row`).
/// The table handles stay valid only while the function container lives.
pub(crate) fn open_outputs(
    sdk: &ffi::SapNwRfc,
    func_desc: RFC_FUNCTION_DESC_HANDLE,
    func_handle: RFC_FUNCTION_HANDLE,
    compat: CompatOptions,
) -> Result<IndexMap<String, OutputSlot>, RfcError> {
    let param_count = get_parameter_count(sdk, func_desc)?;
    let mut result = IndexMap::with_capacity(param_count as usize);
    for_each_output(
        sdk,
        func_desc,
        func_handle,
        compat,
        |name, name_uc, param_desc| {
            let slot = if param_desc.type_ == RFCTYPE::RFCTYPE_TABLE {
                let (handle, rows) = open_table(sdk, func_handle, name_uc)?;
                OutputSlot::Table {
                    handle,
                    type_desc: param_desc.typeDescHandle,
                    rows,
                }
            } else {
                OutputSlot::Value(read_field(sdk, func_handle, name_uc, param_desc)?)
            };
            result.insert(name, slot);
            Ok(())
        },
    )?;
    Ok(result)
}

/// Visit every parameter `read_outputs` returns, in descriptor order:
/// IMPORT parameters and inactive parameters are skipped unless the compat
/// flags ask for them. The one place that decides what an output is.
fn for_each_output(
    sdk: &ffi::SapNwRfc,
    func_desc: RFC_FUNCTION_DESC_HANDLE,
    func_handle: RFC_FUNCTION_HANDLE,
    compat: CompatOptions,
    mut visit: impl FnMut(String, &[u16], &RFC_PARAMETER_DESC) -> Result<(), RfcError>,
) -> Result<(), RfcError> {
    let param_count = get_parameter_count(sdk, func_desc)?;

    for i in 0..param_count {
        let mut param_desc: RFC_PARAMETER_DESC = unsafe { std::mem::zeroed() };
        let mut error_info = ffi::zero_error_info();

        // Safety: func_desc is valid, i is in range.
        let rc = unsafe {
            sdk.RfcGetParameterDescByIndex(func_desc, i, &mut param_desc, &mut error_info)
        };
        if rc != RFC_RC::RFC_OK {
            let info = ffi::extract_error_info("RfcGetParameterDescByIndex", &error_info);
            return Err(RfcError::from_error_info(info));
        }

        // Skip IMPORT-only (direction 0x01), unless compat asks for
        // pyrfc's echo (`CompatOptions::return_import_params`).
        if param_desc.direction == RFC_DIRECTION::RFC_IMPORT && !compat.return_import_params {
            continue;
        }

        // A name decoded from a descriptor stops at the first NUL, so the
        // check cannot fail here.
        let name = uc::decode(&param_desc.name);
        let name_uc = uc::encode_nul_terminated(&name, || "parameter name".into())?;

        // Skip parameters deactivated for this call — the backend
        // sent no data for them (guide §4.2.2), so reporting a value here
        // would invent one. The SDK's own flag is the authority, not the
        // caller's inactive list (spelling-safe).
        let mut is_active: std::ffi::c_int = 1;
        // Safety: func_handle is valid, name_uc is null-terminated,
        // is_active and error_info are valid out-params.
        let rc = unsafe {
            sdk.RfcIsParameterActive(
                func_handle,
                name_uc.as_ptr(),
                &mut is_active,
                &mut error_info,
            )
        };
        if rc != RFC_RC::RFC_OK {
            let info = ffi::extract_error_info("RfcIsParameterActive", &error_info);
            return Err(RfcError::from_error_info(info));
        }
        // Compat-only: `CompatOptions::read_inactive_outputs`.
        if is_active == 0 && !compat.read_inactive_outputs {
            continue;
        }

        visit(name, &name_uc, &param_desc)?;
    }

    Ok(())
}

/// Read ALL parameters (including IMPORT) from a function handle.
/// For offline round-trip testing only — in production, use `read_outputs`.
pub(crate) fn read_all(
    sdk: &ffi::SapNwRfc,
    func_desc: RFC_FUNCTION_DESC_HANDLE,
    func_handle: RFC_FUNCTION_HANDLE,
) -> Result<IndexMap<String, RfcValue>, RfcError> {
    let param_count = get_parameter_count(sdk, func_desc)?;
    let mut result = IndexMap::with_capacity(param_count as usize);

    for i in 0..param_count {
        let mut param_desc: RFC_PARAMETER_DESC = unsafe { std::mem::zeroed() };
        let mut error_info = ffi::zero_error_info();

        // Safety: func_desc is valid, i is in range.
        let rc = unsafe {
            sdk.RfcGetParameterDescByIndex(func_desc, i, &mut param_desc, &mut error_info)
        };
        if rc != RFC_RC::RFC_OK {
            let info = ffi::extract_error_info("RfcGetParameterDescByIndex", &error_info);
            return Err(RfcError::from_error_info(info));
        }

        // A name decoded from a descriptor stops at the first NUL, so the
        // check cannot fail here.
        let name = uc::decode(&param_desc.name);
        let name_uc = uc::encode_nul_terminated(&name, || "parameter name".into())?;
        let value = read_field(sdk, func_handle, &name_uc, &param_desc)?;
        result.insert(name, value);
    }

    Ok(result)
}

// ── read_field ─────────────────────────────────────────────────────────

/// Read a single field value from a data container, dispatching by RFCTYPE.
fn read_field(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    desc: &impl FieldLike,
) -> Result<RfcValue, RfcError> {
    match desc.rfctype() {
        RFCTYPE::RFCTYPE_CHAR => read_char(sdk, handle, name_uc, desc.uc_length()),
        RFCTYPE::RFCTYPE_STRING => read_string(sdk, handle, name_uc),
        RFCTYPE::RFCTYPE_NUM => read_numc(sdk, handle, name_uc, desc.uc_length()),
        RFCTYPE::RFCTYPE_BCD | RFCTYPE::RFCTYPE_DECF16 | RFCTYPE::RFCTYPE_DECF34 => {
            read_decimal_via_string(sdk, handle, name_uc)
        }
        RFCTYPE::RFCTYPE_FLOAT => read_float(sdk, handle, name_uc),
        RFCTYPE::RFCTYPE_INT1 => read_int1(sdk, handle, name_uc),
        RFCTYPE::RFCTYPE_INT2 => read_int2(sdk, handle, name_uc),
        RFCTYPE::RFCTYPE_INT => read_int4(sdk, handle, name_uc),
        RFCTYPE::RFCTYPE_INT8 => read_int8(sdk, handle, name_uc),
        RFCTYPE::RFCTYPE_DATE => read_date(sdk, handle, name_uc),
        RFCTYPE::RFCTYPE_TIME => read_time(sdk, handle, name_uc),
        RFCTYPE::RFCTYPE_BYTE => read_bytes(sdk, handle, name_uc, desc.uc_length()),
        RFCTYPE::RFCTYPE_XSTRING => read_xstring(sdk, handle, name_uc),
        t if is_temporal(t) => read_temporal_via_string(sdk, handle, name_uc),
        RFCTYPE::RFCTYPE_STRUCTURE => read_structure(sdk, handle, name_uc, desc.type_desc_handle()),
        RFCTYPE::RFCTYPE_TABLE => read_table(sdk, handle, name_uc, desc.type_desc_handle()),
        other => Err(RfcError::Other(RfcErrorInfo {
            message: format!("unsupported RFCTYPE {}", other.0),
            ..Default::default()
        })),
    }
}

// ── set_field ──────────────────────────────────────────────────────────

/// Write a single RfcValue into a data container field. Applies the compat
/// empty DATE/TIME skip and the length check recursively (structures/tables thread `compat`
/// and `path` through).
fn set_field(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    value: &RfcValue,
    desc: &impl FieldLike,
    compat: CompatOptions,
    path: &str,
) -> Result<(), RfcError> {
    // Length check before any setter call.
    check_input_length(value, desc.rfctype(), desc.uc_length(), path, compat)?;

    match (value, desc.rfctype()) {
        // ── integer targets: integer-carrying variants only ─
        (
            v,
            t @ (RFCTYPE::RFCTYPE_INT1
            | RFCTYPE::RFCTYPE_INT2
            | RFCTYPE::RFCTYPE_INT
            | RFCTYPE::RFCTYPE_INT8),
        ) => match int_value(v) {
            Some(n) => set_int_field(sdk, handle, name_uc, n, t, path, compat),
            None => Err(RfcError::InvalidType(format!(
                "{path}: {} field requires int input, got {}",
                rfctype_name(t),
                variant_name(v)
            ))),
        },

        // ── typed float path ───────────────────────────────────────
        (RfcValue::Float(f), RFCTYPE::RFCTYPE_FLOAT) => set_float(sdk, handle, name_uc, *f),

        // ── binary paths ───────────────────────────────────────────
        (RfcValue::Bytes(b), RFCTYPE::RFCTYPE_BYTE) => set_bytes(sdk, handle, name_uc, b),
        (RfcValue::Bytes(b), RFCTYPE::RFCTYPE_XSTRING) => set_xstring(sdk, handle, name_uc, b),

        // ── structure/table ───────────────────────────────────────
        (RfcValue::Structure(map), RFCTYPE::RFCTYPE_STRUCTURE) => set_structure(
            sdk,
            handle,
            name_uc,
            map,
            desc.type_desc_handle(),
            compat,
            path,
        ),
        (RfcValue::Table(rows), RFCTYPE::RFCTYPE_TABLE) => set_table(
            sdk,
            handle,
            name_uc,
            rows,
            desc.type_desc_handle(),
            compat,
            path,
        ),

        // ── integer values: valid for numeric text targets only ────
        // (the decimal rendering of an integer is unambiguous; the
        // conversion authority stays with the SDK via RfcSetString.)
        (
            RfcValue::Int(_)
            | RfcValue::Int1(_)
            | RfcValue::Int2(_)
            | RfcValue::Int4(_)
            | RfcValue::Int8(_),
            RFCTYPE::RFCTYPE_NUM
            | RFCTYPE::RFCTYPE_BCD
            | RFCTYPE::RFCTYPE_DECF16
            | RFCTYPE::RFCTYPE_DECF34
            | RFCTYPE::RFCTYPE_FLOAT,
        ) => {
            let s = value_to_string(value);
            set_string(sdk, handle, name_uc, &s)
        }
        (
            RfcValue::Int(_)
            | RfcValue::Int1(_)
            | RfcValue::Int2(_)
            | RfcValue::Int4(_)
            | RfcValue::Int8(_),
            t,
        ) => Err(RfcError::InvalidType(format!(
            "{path}: int input is not valid for {} field",
            rfctype_name(t)
        ))),

        // ── type mismatches (InvalidType → TypeError) ────────
        (_, t @ (RFCTYPE::RFCTYPE_BYTE | RFCTYPE::RFCTYPE_XSTRING)) => {
            Err(RfcError::InvalidType(format!(
                "{path}: {} field requires bytes input, got {}",
                rfctype_name(t),
                variant_name(value)
            )))
        }
        (_, t @ (RFCTYPE::RFCTYPE_STRUCTURE | RFCTYPE::RFCTYPE_TABLE)) => {
            Err(RfcError::InvalidType(format!(
                "{path}: {} field requires matching input, got {}",
                rfctype_name(t),
                variant_name(value)
            )))
        }

        // ── container/bytes values on scalar targets: type mismatch ─
        // (guards the string fallback — value_to_string can't render these)
        (RfcValue::Bytes(_) | RfcValue::Structure(_) | RfcValue::Table(_), t) => {
            Err(RfcError::InvalidType(format!(
                "{path}: {} field cannot take {} input",
                rfctype_name(t),
                variant_name(value)
            )))
        }

        // ── compat: pyrfc's fixed-width DATE/TIME setters ──────────
        // pyrfc 3.4 sets DATE/TIME with RfcSetDate/RfcSetTime, which read
        // exactly 8/6 characters. See `CompatOptions::pyrfc_date_time_setters`.
        (
            RfcValue::String(s) | RfcValue::Date(s) | RfcValue::Time(s),
            t @ (RFCTYPE::RFCTYPE_DATE | RFCTYPE::RFCTYPE_TIME),
        ) if compat.pyrfc_date_time_setters => set_date_time_fixed(sdk, handle, name_uc, s, t),

        // ── decimal targets: reject non-numeric input ───────
        // RfcSetString silently garbage-converts a non-numeric string for a
        // BCD field ("A" → 1.00), so validate before the setter. Skipped in
        // compat mode, where the pyrfc-exact pre-pass owns this.
        // (Int variants already handled above; Float/Decimal render to valid
        // decimal strings, so only a bad String is ever rejected here.)
        (_, t @ (RFCTYPE::RFCTYPE_BCD | RFCTYPE::RFCTYPE_DECF16 | RFCTYPE::RFCTYPE_DECF34))
            if !compat.skip_decimal_input_check =>
        {
            let s = value_to_string(value);
            if !is_valid_decimal(&s) {
                return Err(RfcError::InvalidInput(format!(
                    "{path}: {} field requires a numeric value, got {s:?}",
                    rfctype_name(t)
                )));
            }
            set_string(sdk, handle, name_uc, &s)
        }

        // ── string fallback for the remaining scalar targets ───────
        // Compat never reaches this arm with an empty DATE/TIME: the
        // compat skip bypasses the setter and the arm above takes every DATE/TIME string.
        _ => {
            let s = empty_date_time_as_initial(value_to_string(value), desc.rfctype());
            set_string(sdk, handle, name_uc, &s)
        }
    }
}

/// Native: an empty string for a DATE or TIME target becomes the type's
/// initial value; every other input is returned unchanged.
///
/// Trap: `RfcSetString` rejects an empty DATE/TIME before SDK 7.50 PL 18
/// and sets the initial value from PL 18. Sending the initial value gives
/// the PL 18 result on every patch level.
fn empty_date_time_as_initial(s: String, rfctype: RFCTYPE) -> String {
    match rfctype {
        RFCTYPE::RFCTYPE_DATE if s.is_empty() => "00000000".to_owned(),
        RFCTYPE::RFCTYPE_TIME if s.is_empty() => "000000".to_owned(),
        _ => s,
    }
}

/// Set an integer into an INTx field.
///
/// Native: range-check against the target width, then the width-correct
/// typed setter — we never narrow or wrap in our own code; a range
/// violation is an error, not a conversion.
///
/// Compat (`pyrfc_int_setters`): mirror pyrfc 3.4's `fillVariable` wire
/// behavior — INT/INT1/INT2 all via `RfcSetInt` whenever the value fits
/// i32 (field-range overflow like 256→INT1 is the SDK's to judge), INT8
/// via `RfcSetInt8` when it fits i64. Values beyond the C-cast range
/// raise `InvalidInput` where pyrfc raised Cython's `OverflowError` —
/// a known class divergence.
fn set_int_field(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    n: i128,
    rfctype: RFCTYPE,
    path: &str,
    compat: CompatOptions,
) -> Result<(), RfcError> {
    match narrow_int(n, rfctype, path, compat)? {
        NarrowedInt::I1(v) => set_int1(sdk, handle, name_uc, v),
        NarrowedInt::I2(v) => set_int2(sdk, handle, name_uc, v),
        NarrowedInt::I4(v) => set_int4(sdk, handle, name_uc, v),
        NarrowedInt::I8(v) => set_int8(sdk, handle, name_uc, v),
    }
}

/// The width a `set_int_field` value narrows to — determines the setter.
#[derive(Debug)]
enum NarrowedInt {
    I1(u8),
    I2(i16),
    I4(i32),
    I8(i64),
}

/// Pure narrowing step of `set_int_field` — see there for semantics.
fn narrow_int(
    n: i128,
    rfctype: RFCTYPE,
    path: &str,
    compat: CompatOptions,
) -> Result<NarrowedInt, RfcError> {
    if compat.pyrfc_int_setters {
        return match rfctype {
            RFCTYPE::RFCTYPE_INT8 => i64::try_from(n).map(NarrowedInt::I8).map_err(|_| {
                int_range_error(n, "INT8", "-9223372036854775808..9223372036854775807", path)
            }),
            // INT, INT1, INT2 — pyrfc uses RfcSetInt (32-bit) for all three.
            _ => i32::try_from(n).map(NarrowedInt::I4).map_err(|_| {
                int_range_error(
                    n,
                    "the 32-bit pyrfc INT/INT1/INT2 setter path",
                    "-2147483648..2147483647",
                    path,
                )
            }),
        };
    }
    match rfctype {
        RFCTYPE::RFCTYPE_INT1 => u8::try_from(n)
            .map(NarrowedInt::I1)
            .map_err(|_| int_range_error(n, "INT1", "0..255", path)),
        RFCTYPE::RFCTYPE_INT2 => i16::try_from(n)
            .map(NarrowedInt::I2)
            .map_err(|_| int_range_error(n, "INT2", "-32768..32767", path)),
        RFCTYPE::RFCTYPE_INT => i32::try_from(n)
            .map(NarrowedInt::I4)
            .map_err(|_| int_range_error(n, "INT4", "-2147483648..2147483647", path)),
        RFCTYPE::RFCTYPE_INT8 => i64::try_from(n).map(NarrowedInt::I8).map_err(|_| {
            int_range_error(n, "INT8", "-9223372036854775808..9223372036854775807", path)
        }),
        other => unreachable!("narrow_int called with non-integer RFCTYPE {}", other.0),
    }
}

fn int_range_error(n: i128, target: &str, range: &str, path: &str) -> RfcError {
    RfcError::InvalidInput(format!(
        "input for {path}: {n} out of range for {target} ({range})"
    ))
}

/// Human-readable RFCTYPE name for error messages.
fn rfctype_name(t: RFCTYPE) -> &'static str {
    match t {
        RFCTYPE::RFCTYPE_CHAR => "CHAR",
        RFCTYPE::RFCTYPE_STRING => "STRING",
        RFCTYPE::RFCTYPE_NUM => "NUMC",
        RFCTYPE::RFCTYPE_BCD => "BCD",
        RFCTYPE::RFCTYPE_DECF16 => "DECF16",
        RFCTYPE::RFCTYPE_DECF34 => "DECF34",
        RFCTYPE::RFCTYPE_FLOAT => "FLOAT",
        RFCTYPE::RFCTYPE_INT1 => "INT1",
        RFCTYPE::RFCTYPE_INT2 => "INT2",
        RFCTYPE::RFCTYPE_INT => "INT4",
        RFCTYPE::RFCTYPE_INT8 => "INT8",
        RFCTYPE::RFCTYPE_DATE => "DATE",
        RFCTYPE::RFCTYPE_TIME => "TIME",
        RFCTYPE::RFCTYPE_BYTE => "BYTE",
        RFCTYPE::RFCTYPE_XSTRING => "XSTRING",
        RFCTYPE::RFCTYPE_STRUCTURE => "STRUCTURE",
        RFCTYPE::RFCTYPE_TABLE => "TABLE",
        t if is_temporal(t) => "temporal",
        _ => "unsupported-RFCTYPE",
    }
}

fn variant_name(v: &RfcValue) -> &'static str {
    match v {
        RfcValue::Char(_) => "Char",
        RfcValue::String(_) => "String",
        RfcValue::Numc(_) => "Numc",
        RfcValue::Decimal(_) => "Decimal",
        RfcValue::Float(_) => "Float",
        RfcValue::Int(_) => "Int",
        RfcValue::Int1(_) => "Int1",
        RfcValue::Int2(_) => "Int2",
        RfcValue::Int4(_) => "Int4",
        RfcValue::Int8(_) => "Int8",
        RfcValue::Date(_) => "Date",
        RfcValue::Time(_) => "Time",
        RfcValue::Bytes(_) => "Bytes",
        RfcValue::Temporal(_) => "Temporal",
        RfcValue::Structure(_) => "Structure",
        RfcValue::Table(_) => "Table",
    }
}

fn value_to_string(v: &RfcValue) -> String {
    match v {
        RfcValue::Char(s)
        | RfcValue::String(s)
        | RfcValue::Numc(s)
        | RfcValue::Decimal(s)
        | RfcValue::Date(s)
        | RfcValue::Time(s)
        | RfcValue::Temporal(s) => s.clone(),
        RfcValue::Float(f) => f.to_string(),
        RfcValue::Int(n) => n.to_string(),
        RfcValue::Int1(n) => n.to_string(),
        RfcValue::Int2(n) => n.to_string(),
        RfcValue::Int4(n) => n.to_string(),
        RfcValue::Int8(n) => n.to_string(),
        RfcValue::Bytes(_) | RfcValue::Structure(_) | RfcValue::Table(_) => {
            unreachable!("handled by typed paths above")
        }
    }
}

// ── temporal type check ────────────────────────────────────────────────

fn is_temporal(t: RFCTYPE) -> bool {
    (32..=40).contains(&t.0)
}

// ── helper: check SDK return code ──────────────────────────────────────

fn check_rc(
    sdk_function: &'static str,
    rc: RFC_RC,
    error_info: &ffi::RFC_ERROR_INFO,
) -> Result<(), RfcError> {
    if rc != RFC_RC::RFC_OK {
        let info = ffi::extract_error_info(sdk_function, error_info);
        return Err(RfcError::from_error_info(info));
    }
    Ok(())
}

// ── helper: parameter count ────────────────────────────────────────────

fn get_parameter_count(
    sdk: &ffi::SapNwRfc,
    func_desc: RFC_FUNCTION_DESC_HANDLE,
) -> Result<u32, RfcError> {
    let mut count: std::ffi::c_uint = 0;
    let mut error_info = ffi::zero_error_info();
    // Safety: func_desc is a valid function descriptor handle.
    let rc = unsafe { sdk.RfcGetParameterCount(func_desc, &mut count, &mut error_info) };
    check_rc("RfcGetParameterCount", rc, &error_info)?;
    Ok(count)
}

// ── scalar getters ─────────────────────────────────────────────────────

fn read_char(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    uc_length: u32,
) -> Result<RfcValue, RfcError> {
    // ucLength is in bytes; each SAP_UC is 2 bytes.
    let char_count = (uc_length / 2) as usize;
    let mut buf = vec![0u16; char_count];
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated, buf is large enough.
    let rc = unsafe {
        sdk.RfcGetChars(
            handle,
            name_uc.as_ptr(),
            buf.as_mut_ptr(),
            char_count as std::ffi::c_uint,
            &mut error_info,
        )
    };
    check_rc("RfcGetChars", rc, &error_info)?;
    Ok(RfcValue::Char(uc::decode_len(&buf, char_count)))
}

fn read_string(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<RfcValue, RfcError> {
    let mut str_len: std::ffi::c_uint = 0;
    let mut error_info = ffi::zero_error_info();

    // First call: get the length.
    // Safety: handle is valid, name_uc is null-terminated.
    let rc =
        unsafe { sdk.RfcGetStringLength(handle, name_uc.as_ptr(), &mut str_len, &mut error_info) };
    check_rc("RfcGetStringLength", rc, &error_info)?;

    // Allocate buffer (length + 1 for null terminator).
    let buf_len = str_len as usize + 1;
    let mut buf = vec![0u16; buf_len];
    let mut result_len: std::ffi::c_uint = 0;
    error_info = ffi::zero_error_info();

    // Safety: buf is large enough (str_len + 1).
    let rc = unsafe {
        sdk.RfcGetString(
            handle,
            name_uc.as_ptr(),
            buf.as_mut_ptr(),
            buf_len as std::ffi::c_uint,
            &mut result_len,
            &mut error_info,
        )
    };
    check_rc("RfcGetString", rc, &error_info)?;
    Ok(RfcValue::String(uc::decode_len(&buf, result_len as usize)))
}

fn read_numc(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    uc_length: u32,
) -> Result<RfcValue, RfcError> {
    let char_count = (uc_length / 2) as usize;
    let mut buf = vec![0u16; char_count];
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated, buf is large enough.
    let rc = unsafe {
        sdk.RfcGetNum(
            handle,
            name_uc.as_ptr(),
            buf.as_mut_ptr(),
            char_count as std::ffi::c_uint,
            &mut error_info,
        )
    };
    check_rc("RfcGetNum", rc, &error_info)?;
    Ok(RfcValue::Numc(uc::decode_len(&buf, char_count)))
}

fn read_decimal_via_string(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<RfcValue, RfcError> {
    // BCD/DECF read via RfcGetString — the SDK formats the decimal.
    // The result may be EMPTY (rc=OK) for invalid packed content —
    // the core keeps it verbatim; the converters decide (native: NaN,
    // compat: pyrfc's InvalidOperation).
    let s = get_string_value(sdk, handle, name_uc)?;
    Ok(RfcValue::Decimal(s))
}

fn read_float(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<RfcValue, RfcError> {
    let mut val: f64 = 0.0;
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated, val is a valid out-param.
    let rc = unsafe { sdk.RfcGetFloat(handle, name_uc.as_ptr(), &mut val, &mut error_info) };
    check_rc("RfcGetFloat", rc, &error_info)?;
    Ok(RfcValue::Float(val))
}

fn read_int1(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<RfcValue, RfcError> {
    let mut val: u8 = 0;
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc = unsafe { sdk.RfcGetInt1(handle, name_uc.as_ptr(), &mut val, &mut error_info) };
    check_rc("RfcGetInt1", rc, &error_info)?;
    Ok(RfcValue::Int1(val))
}

fn read_int2(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<RfcValue, RfcError> {
    let mut val: i16 = 0;
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc = unsafe { sdk.RfcGetInt2(handle, name_uc.as_ptr(), &mut val, &mut error_info) };
    check_rc("RfcGetInt2", rc, &error_info)?;
    Ok(RfcValue::Int2(val))
}

fn read_int4(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<RfcValue, RfcError> {
    let mut val: std::ffi::c_int = 0;
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc = unsafe { sdk.RfcGetInt(handle, name_uc.as_ptr(), &mut val, &mut error_info) };
    check_rc("RfcGetInt", rc, &error_info)?;
    Ok(RfcValue::Int4(val))
}

fn read_int8(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<RfcValue, RfcError> {
    let mut val: std::ffi::c_longlong = 0;
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc = unsafe { sdk.RfcGetInt8(handle, name_uc.as_ptr(), &mut val, &mut error_info) };
    check_rc("RfcGetInt8", rc, &error_info)?;
    Ok(RfcValue::Int8(val))
}

fn read_date(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<RfcValue, RfcError> {
    let mut buf = [0u16; 8];
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated, buf is RFC_DATE sized.
    let rc = unsafe { sdk.RfcGetDate(handle, name_uc.as_ptr(), buf.as_mut_ptr(), &mut error_info) };
    check_rc("RfcGetDate", rc, &error_info)?;
    Ok(RfcValue::Date(uc::decode_len(&buf, 8)))
}

fn read_time(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<RfcValue, RfcError> {
    let mut buf = [0u16; 6];
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated, buf is RFC_TIME sized.
    let rc = unsafe { sdk.RfcGetTime(handle, name_uc.as_ptr(), buf.as_mut_ptr(), &mut error_info) };
    check_rc("RfcGetTime", rc, &error_info)?;
    Ok(RfcValue::Time(uc::decode_len(&buf, 6)))
}

fn read_bytes(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    uc_length: u32,
) -> Result<RfcValue, RfcError> {
    // For BYTE (fixed-length raw), ucLength = byte length.
    let byte_len = uc_length as usize;
    let mut buf = vec![0u8; byte_len];
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated, buf is large enough.
    let rc = unsafe {
        sdk.RfcGetBytes(
            handle,
            name_uc.as_ptr(),
            buf.as_mut_ptr(),
            byte_len as std::ffi::c_uint,
            &mut error_info,
        )
    };
    check_rc("RfcGetBytes", rc, &error_info)?;
    Ok(RfcValue::Bytes(buf))
}

fn read_xstring(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<RfcValue, RfcError> {
    // Use RfcGetStringLength to get the byte length first — the SDK
    // returns "Value is null" if RfcGetXString is called with a null
    // buffer pointer, even with length 0.
    let mut byte_len: std::ffi::c_uint = 0;
    let mut error_info = ffi::zero_error_info();

    // Safety: handle is valid, name_uc is null-terminated.
    let rc =
        unsafe { sdk.RfcGetStringLength(handle, name_uc.as_ptr(), &mut byte_len, &mut error_info) };
    check_rc("RfcGetStringLength", rc, &error_info)?;

    if byte_len == 0 {
        return Ok(RfcValue::Bytes(Vec::new()));
    }

    let mut buf = vec![0u8; byte_len as usize];
    let mut result_len: std::ffi::c_uint = 0;
    error_info = ffi::zero_error_info();

    // Safety: buf is large enough (byte_len bytes).
    let rc = unsafe {
        sdk.RfcGetXString(
            handle,
            name_uc.as_ptr(),
            buf.as_mut_ptr(),
            byte_len,
            &mut result_len,
            &mut error_info,
        )
    };
    check_rc("RfcGetXString", rc, &error_info)?;
    buf.truncate(result_len as usize);
    Ok(RfcValue::Bytes(buf))
}

fn read_temporal_via_string(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<RfcValue, RfcError> {
    // Temporal types (UTCLONG..CDAY) read via RfcGetString.
    let s = get_string_value(sdk, handle, name_uc)?;
    Ok(RfcValue::Temporal(s))
}

// ── structure/table readers (B6 completes, but scaffolded here) ────────

fn read_structure(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    type_desc: RFC_TYPE_DESC_HANDLE,
) -> Result<RfcValue, RfcError> {
    let mut struct_handle: RFC_STRUCTURE_HANDLE = std::ptr::null_mut();
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc = unsafe {
        sdk.RfcGetStructure(
            handle,
            name_uc.as_ptr(),
            &mut struct_handle,
            &mut error_info,
        )
    };
    check_rc("RfcGetStructure", rc, &error_info)?;

    read_structure_fields(sdk, struct_handle, type_desc)
}

fn read_structure_fields(
    sdk: &ffi::SapNwRfc,
    struct_handle: RFC_STRUCTURE_HANDLE,
    type_desc: RFC_TYPE_DESC_HANDLE,
) -> Result<RfcValue, RfcError> {
    read_structure_map(sdk, struct_handle, type_desc).map(RfcValue::Structure)
}

fn read_structure_map(
    sdk: &ffi::SapNwRfc,
    struct_handle: RFC_STRUCTURE_HANDLE,
    type_desc: RFC_TYPE_DESC_HANDLE,
) -> Result<IndexMap<String, RfcValue>, RfcError> {
    let field_count = get_field_count(sdk, type_desc)?;
    let mut map = IndexMap::with_capacity(field_count as usize);

    for i in 0..field_count {
        let mut field_desc: RFC_FIELD_DESC = unsafe { std::mem::zeroed() };
        let mut error_info = ffi::zero_error_info();
        // Safety: type_desc is valid, i is in range.
        let rc =
            unsafe { sdk.RfcGetFieldDescByIndex(type_desc, i, &mut field_desc, &mut error_info) };
        check_rc("RfcGetFieldDescByIndex", rc, &error_info)?;

        // A name decoded from a descriptor stops at the first NUL, so the
        // check cannot fail here.
        let name = uc::decode(&field_desc.name);
        let name_uc = uc::encode_nul_terminated(&name, || "field name".into())?;
        let value = read_field(sdk, struct_handle, &name_uc, &field_desc)?;
        map.insert(name, value);
    }

    Ok(map)
}

fn read_table(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    type_desc: RFC_TYPE_DESC_HANDLE,
) -> Result<RfcValue, RfcError> {
    let (table_handle, row_count) = open_table(sdk, handle, name_uc)?;
    read_rows(sdk, table_handle, type_desc, 0..row_count).map(RfcValue::Table)
}

/// Get a table parameter or field and its row count.
fn open_table(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<(RFC_TABLE_HANDLE, u32), RfcError> {
    let mut table_handle: RFC_TABLE_HANDLE = std::ptr::null_mut();
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc =
        unsafe { sdk.RfcGetTable(handle, name_uc.as_ptr(), &mut table_handle, &mut error_info) };
    check_rc("RfcGetTable", rc, &error_info)?;
    let row_count = get_row_count(sdk, table_handle)?;
    Ok((table_handle, row_count))
}

/// Read the rows `range` of a table, in index order. Each row is a
/// structure of `type_desc`; nested tables in a row are read in full.
pub(crate) fn read_rows(
    sdk: &ffi::SapNwRfc,
    table_handle: RFC_TABLE_HANDLE,
    type_desc: RFC_TYPE_DESC_HANDLE,
    range: std::ops::Range<u32>,
) -> Result<Vec<IndexMap<String, RfcValue>>, RfcError> {
    let mut rows = Vec::with_capacity(range.len());

    for i in range {
        let mut error_info = ffi::zero_error_info();
        // Safety: table_handle is valid, i is in range.
        let rc = unsafe { sdk.RfcMoveTo(table_handle, i, &mut error_info) };
        check_rc("RfcMoveTo", rc, &error_info)?;

        error_info = ffi::zero_error_info();
        // Safety: table_handle is valid, positioned on a row.
        let row_handle = unsafe { sdk.RfcGetCurrentRow(table_handle, &mut error_info) };
        if row_handle.is_null() {
            let info = ffi::extract_error_info("RfcGetCurrentRow", &error_info);
            return Err(RfcError::from_error_info(info));
        }

        rows.push(read_structure_map(sdk, row_handle, type_desc)?);
    }

    Ok(rows)
}

/// Delete the last row of a table (`RfcMoveToLastRow` +
/// `RfcDeleteCurrentRow`). The SDK releases most of the row's storage at once, so
/// deleting each row after it is read keeps the container shrinking while
/// a large table is converted. Row handles obtained earlier for that row
/// are invalid afterwards.
pub(crate) fn delete_last_row(
    sdk: &ffi::SapNwRfc,
    table_handle: RFC_TABLE_HANDLE,
) -> Result<(), RfcError> {
    let mut error_info = ffi::zero_error_info();
    // Safety: table_handle is a valid table handle; error_info is a valid
    // out-param. The SDK errors on an empty table.
    let rc = unsafe { sdk.RfcMoveToLastRow(table_handle, &mut error_info) };
    check_rc("RfcMoveToLastRow", rc, &error_info)?;

    error_info = ffi::zero_error_info();
    // Safety: table_handle is valid and positioned on its last row.
    let rc = unsafe { sdk.RfcDeleteCurrentRow(table_handle, &mut error_info) };
    check_rc("RfcDeleteCurrentRow", rc, &error_info)
}

// ── scalar setters ─────────────────────────────────────────────────────

fn set_string(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    value: &str,
) -> Result<(), RfcError> {
    // A value goes with an explicit length, so an embedded NUL reaches the
    // SDK: no NUL check here (only names and settings are checked).
    let value_uc = uc::encode_unchecked(value);
    // The buffer ends with a null terminator; the length parameter excludes it.
    let len = (value_uc.len() - 1) as std::ffi::c_uint;
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc and value_uc are null-terminated.
    let rc = unsafe {
        sdk.RfcSetString(
            handle,
            name_uc.as_ptr(),
            value_uc.as_ptr(),
            len,
            &mut error_info,
        )
    };
    check_rc("RfcSetString", rc, &error_info)
}

/// COMPAT-ONLY: set a DATE or TIME field through the SDK's fixed-width
/// setter, as pyrfc 3.4 does. The value is cut to the field width (8 or 6
/// UTF-16 units) and a shorter value is padded with NUL units. pyrfc passes
/// its NUL-terminated buffer, so a value one unit short reads the NUL
/// terminator; shorter values make pyrfc read past its buffer, where we pad
/// with NUL instead.
fn set_date_time_fixed(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    value: &str,
    rfctype: RFCTYPE,
) -> Result<(), RfcError> {
    let width = if rfctype == RFCTYPE::RFCTYPE_DATE {
        8
    } else {
        6
    };
    let mut buf: Vec<u16> = value.encode_utf16().take(width).collect();
    buf.resize(width, 0);
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated, buf holds exactly
    // the `width` units that RfcSetDate (8) / RfcSetTime (6) read.
    let rc = unsafe {
        if rfctype == RFCTYPE::RFCTYPE_DATE {
            sdk.RfcSetDate(handle, name_uc.as_ptr(), buf.as_ptr(), &mut error_info)
        } else {
            sdk.RfcSetTime(handle, name_uc.as_ptr(), buf.as_ptr(), &mut error_info)
        }
    };
    let setter = if rfctype == RFCTYPE::RFCTYPE_DATE {
        "RfcSetDate"
    } else {
        "RfcSetTime"
    };
    check_rc(setter, rc, &error_info)
}

fn set_float(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    value: f64,
) -> Result<(), RfcError> {
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc = unsafe { sdk.RfcSetFloat(handle, name_uc.as_ptr(), value, &mut error_info) };
    check_rc("RfcSetFloat", rc, &error_info)
}

fn set_int1(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    value: u8,
) -> Result<(), RfcError> {
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc = unsafe { sdk.RfcSetInt1(handle, name_uc.as_ptr(), value, &mut error_info) };
    check_rc("RfcSetInt1", rc, &error_info)
}

fn set_int2(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    value: i16,
) -> Result<(), RfcError> {
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc = unsafe { sdk.RfcSetInt2(handle, name_uc.as_ptr(), value, &mut error_info) };
    check_rc("RfcSetInt2", rc, &error_info)
}

fn set_int4(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    value: i32,
) -> Result<(), RfcError> {
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc = unsafe { sdk.RfcSetInt(handle, name_uc.as_ptr(), value, &mut error_info) };
    check_rc("RfcSetInt", rc, &error_info)
}

fn set_int8(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    value: i64,
) -> Result<(), RfcError> {
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc = unsafe { sdk.RfcSetInt8(handle, name_uc.as_ptr(), value, &mut error_info) };
    check_rc("RfcSetInt8", rc, &error_info)
}

fn set_bytes(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    value: &[u8],
) -> Result<(), RfcError> {
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated, value is a valid byte slice.
    let rc = unsafe {
        sdk.RfcSetBytes(
            handle,
            name_uc.as_ptr(),
            value.as_ptr(),
            value.len() as std::ffi::c_uint,
            &mut error_info,
        )
    };
    check_rc("RfcSetBytes", rc, &error_info)
}

fn set_xstring(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    value: &[u8],
) -> Result<(), RfcError> {
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated, value is a valid byte slice.
    let rc = unsafe {
        sdk.RfcSetXString(
            handle,
            name_uc.as_ptr(),
            value.as_ptr(),
            value.len() as std::ffi::c_uint,
            &mut error_info,
        )
    };
    check_rc("RfcSetXString", rc, &error_info)
}

// ── structure/table setters ────────────────────────────────────────────

fn set_structure(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    map: &IndexMap<String, RfcValue>,
    type_desc: RFC_TYPE_DESC_HANDLE,
    compat: CompatOptions,
    path: &str,
) -> Result<(), RfcError> {
    let mut struct_handle: RFC_STRUCTURE_HANDLE = std::ptr::null_mut();
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc = unsafe {
        sdk.RfcGetStructure(
            handle,
            name_uc.as_ptr(),
            &mut struct_handle,
            &mut error_info,
        )
    };
    check_rc("RfcGetStructure", rc, &error_info)?;

    set_structure_fields(sdk, struct_handle, map, type_desc, compat, path)
}

fn set_structure_fields(
    sdk: &ffi::SapNwRfc,
    struct_handle: RFC_STRUCTURE_HANDLE,
    map: &IndexMap<String, RfcValue>,
    type_desc: RFC_TYPE_DESC_HANDLE,
    compat: CompatOptions,
    parent_path: &str,
) -> Result<(), RfcError> {
    for (name, value) in map {
        let name_uc = uc::encode_nul_terminated(name, || format!("field name at {parent_path}"))?;
        let mut field_desc: RFC_FIELD_DESC = unsafe { std::mem::zeroed() };
        let mut error_info = ffi::zero_error_info();

        // Safety: type_desc is valid, name_uc is null-terminated.
        let rc = unsafe {
            sdk.RfcGetFieldDescByName(
                type_desc,
                name_uc.as_ptr(),
                &mut field_desc,
                &mut error_info,
            )
        };
        check_rc("RfcGetFieldDescByName", rc, &error_info)?;

        let field_path = format!("{parent_path}.{name}");

        if skip_input(value, field_desc.type_, compat) {
            continue;
        }

        set_field(
            sdk,
            struct_handle,
            &name_uc,
            value,
            &field_desc,
            compat,
            &field_path,
        )?;
    }
    Ok(())
}

fn set_table(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
    rows: &[IndexMap<String, RfcValue>],
    type_desc: RFC_TYPE_DESC_HANDLE,
    compat: CompatOptions,
    path: &str,
) -> Result<(), RfcError> {
    let mut table_handle: RFC_TABLE_HANDLE = std::ptr::null_mut();
    let mut error_info = ffi::zero_error_info();
    // Safety: handle is valid, name_uc is null-terminated.
    let rc =
        unsafe { sdk.RfcGetTable(handle, name_uc.as_ptr(), &mut table_handle, &mut error_info) };
    check_rc("RfcGetTable", rc, &error_info)?;

    for (i, row_map) in rows.iter().enumerate() {
        error_info = ffi::zero_error_info();
        // Safety: table_handle is valid.
        let row_handle = unsafe { sdk.RfcAppendNewRow(table_handle, &mut error_info) };
        if row_handle.is_null() {
            let info = ffi::extract_error_info("RfcAppendNewRow", &error_info);
            return Err(RfcError::from_error_info(info));
        }
        let row_path = format!("{path}[{i}]");
        set_structure_fields(sdk, row_handle, row_map, type_desc, compat, &row_path)?;
    }
    Ok(())
}

// ── shared string getter (used by decimal and temporal) ────────────────

fn get_string_value(
    sdk: &ffi::SapNwRfc,
    handle: DATA_CONTAINER_HANDLE,
    name_uc: &[u16],
) -> Result<String, RfcError> {
    // RfcGetStringLength only works on STRING/XSTRING fields — for BCD/DECF/
    // temporal targets it fails with "Field not of type STRING or XSTRING".
    // Call RfcGetString with a fixed buffer instead; on RFC_BUFFER_TOO_SMALL
    // the SDK reports the needed length, so grow once and retry.
    // 64 UTF-16 units covers every decimal/temporal form (BCD max 31 digits
    // + sign + separator; DECF34 sci notation ~45; UTCLONG 27).
    let mut buf = vec![0u16; 64];
    let mut result_len: std::ffi::c_uint = 0;
    let mut error_info = ffi::zero_error_info();

    // Safety: handle is valid, name_uc is null-terminated, buf is 64 units.
    let rc = unsafe {
        sdk.RfcGetString(
            handle,
            name_uc.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as std::ffi::c_uint,
            &mut result_len,
            &mut error_info,
        )
    };
    if rc == RFC_RC::RFC_BUFFER_TOO_SMALL {
        buf = vec![0u16; result_len as usize + 1];
        error_info = ffi::zero_error_info();
        // Safety: buf now has the SDK-reported length + 1 for the terminator.
        let rc = unsafe {
            sdk.RfcGetString(
                handle,
                name_uc.as_ptr(),
                buf.as_mut_ptr(),
                buf.len() as std::ffi::c_uint,
                &mut result_len,
                &mut error_info,
            )
        };
        check_rc("RfcGetString", rc, &error_info)?;
    } else {
        check_rc("RfcGetString", rc, &error_info)?;
    }
    Ok(uc::decode_len(&buf, result_len as usize))
}

// ── helper: field/row counts ───────────────────────────────────────────

fn get_field_count(sdk: &ffi::SapNwRfc, type_desc: RFC_TYPE_DESC_HANDLE) -> Result<u32, RfcError> {
    let mut count: std::ffi::c_uint = 0;
    let mut error_info = ffi::zero_error_info();
    // Safety: type_desc is a valid type descriptor handle.
    let rc = unsafe { sdk.RfcGetFieldCount(type_desc, &mut count, &mut error_info) };
    check_rc("RfcGetFieldCount", rc, &error_info)?;
    Ok(count)
}

fn get_row_count(sdk: &ffi::SapNwRfc, table_handle: RFC_TABLE_HANDLE) -> Result<u32, RfcError> {
    let mut count: std::ffi::c_uint = 0;
    let mut error_info = ffi::zero_error_info();
    // Safety: table_handle is a valid table handle.
    let rc = unsafe { sdk.RfcGetRowCount(table_handle, &mut count, &mut error_info) };
    check_rc("RfcGetRowCount", rc, &error_info)?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::CompatOptions;

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
            return_import_params: true,
        }
    }

    // ── check_input_length ────────────────────────────────────────────

    #[test]
    fn char_exact_width_passes() {
        // "ABCD" = 4 UTF-16 units into CHAR(4) (ucLength=8).
        let v = RfcValue::String("ABCD".into());
        assert!(check_input_length(&v, RFCTYPE::RFCTYPE_CHAR, 8, "F", native()).is_ok());
    }

    #[test]
    fn char_over_by_one_errors() {
        // "ABCDE" = 5 UTF-16 units into CHAR(4).
        let v = RfcValue::String("ABCDE".into());
        let err = check_input_length(&v, RFCTYPE::RFCTYPE_CHAR, 8, "PARAM", native()).unwrap_err();
        assert!(matches!(err, RfcError::InvalidInput(_)));
        let msg = err.to_string();
        assert!(msg.contains("5 UTF-16 units"), "{msg}");
        assert!(msg.contains("CHAR(4)"), "{msg}");
        assert!(msg.contains("PARAM"), "{msg}");
    }

    #[test]
    fn char_astral_counting() {
        // "𝄞" = 2 UTF-16 code units → over-long for CHAR(1) (ucLength=2).
        let v = RfcValue::String("𝄞".into());
        let err = check_input_length(&v, RFCTYPE::RFCTYPE_CHAR, 2, "F", native()).unwrap_err();
        assert!(matches!(err, RfcError::InvalidInput(_)));
    }

    #[test]
    fn byte_over_length_errors() {
        // 5 bytes into BYTE(3) (ucLength=3).
        let v = RfcValue::Bytes(vec![1, 2, 3, 4, 5]);
        let err = check_input_length(&v, RFCTYPE::RFCTYPE_BYTE, 3, "F", native()).unwrap_err();
        assert!(matches!(err, RfcError::InvalidInput(_)));
        assert!(err.to_string().contains("BYTE(3)"));
    }

    #[test]
    fn byte_exact_passes() {
        let v = RfcValue::Bytes(vec![1, 2, 3]);
        assert!(check_input_length(&v, RFCTYPE::RFCTYPE_BYTE, 3, "F", native()).is_ok());
    }

    #[test]
    fn string_target_never_checked() {
        let v = RfcValue::String("A".repeat(1000));
        assert!(check_input_length(&v, RFCTYPE::RFCTYPE_STRING, 10, "F", native()).is_ok());
    }

    #[test]
    fn xstring_target_never_checked() {
        let v = RfcValue::Bytes(vec![0; 1000]);
        assert!(check_input_length(&v, RFCTYPE::RFCTYPE_XSTRING, 10, "F", native()).is_ok());
    }

    #[test]
    fn date_never_checked() {
        let v = RfcValue::String("TOOLONG".into());
        assert!(check_input_length(&v, RFCTYPE::RFCTYPE_DATE, 16, "F", native()).is_ok());
    }

    #[test]
    fn time_never_checked() {
        let v = RfcValue::String("TOOLONG".into());
        assert!(check_input_length(&v, RFCTYPE::RFCTYPE_TIME, 12, "F", native()).is_ok());
    }

    #[test]
    fn compat_flag_skips_check() {
        // Over-long, but compat mode → Ok.
        let v = RfcValue::String("ABCDE".into());
        assert!(check_input_length(&v, RFCTYPE::RFCTYPE_CHAR, 8, "F", compat()).is_ok());
    }

    #[test]
    fn numc_over_length_errors() {
        let v = RfcValue::Numc("12345".into());
        let err = check_input_length(&v, RFCTYPE::RFCTYPE_NUM, 8, "F", native()).unwrap_err();
        assert!(matches!(err, RfcError::InvalidInput(_)));
        assert!(err.to_string().contains("NUMC(4)"));
    }

    #[test]
    fn char_input_also_works() {
        let v = RfcValue::Char("AB  ".into());
        assert!(check_input_length(&v, RFCTYPE::RFCTYPE_CHAR, 8, "F", native()).is_ok());
    }

    // ── skip_input ────────────────────────────────────────────────────

    #[test]
    fn skip_empty_date_in_compat() {
        assert!(skip_input(
            &RfcValue::String(String::new()),
            RFCTYPE::RFCTYPE_DATE,
            compat(),
        ));
    }

    #[test]
    fn no_skip_empty_date_in_native() {
        assert!(!skip_input(
            &RfcValue::String(String::new()),
            RFCTYPE::RFCTYPE_DATE,
            native(),
        ));
    }

    #[test]
    fn no_skip_nonempty_date_in_compat() {
        assert!(!skip_input(
            &RfcValue::String("20260714".into()),
            RFCTYPE::RFCTYPE_DATE,
            compat(),
        ));
    }

    #[test]
    fn no_skip_empty_string_for_char_in_compat() {
        assert!(!skip_input(
            &RfcValue::String(String::new()),
            RFCTYPE::RFCTYPE_CHAR,
            compat(),
        ));
    }

    // ── empty_date_time_as_initial ────────────────────────────────────

    #[test]
    fn empty_date_time_becomes_initial_value() {
        let f = |s: &str, t| empty_date_time_as_initial(s.to_owned(), t);
        assert_eq!(f("", RFCTYPE::RFCTYPE_DATE), "00000000");
        assert_eq!(f("", RFCTYPE::RFCTYPE_TIME), "000000");
    }

    #[test]
    fn other_date_time_inputs_unchanged() {
        let f = |s: &str, t| empty_date_time_as_initial(s.to_owned(), t);
        assert_eq!(f("        ", RFCTYPE::RFCTYPE_DATE), "        ");
        assert_eq!(f("2024", RFCTYPE::RFCTYPE_DATE), "2024");
        assert_eq!(f("20260714", RFCTYPE::RFCTYPE_DATE), "20260714");
        assert_eq!(f(" ", RFCTYPE::RFCTYPE_TIME), " ");
        assert_eq!(f("", RFCTYPE::RFCTYPE_CHAR), "");
        assert_eq!(f("", RFCTYPE::RFCTYPE_STRING), "");
        assert_eq!(f("", RFCTYPE::RFCTYPE_NUM), "");
    }

    // ── variant_name includes Char ────────────────────────────────────

    #[test]
    fn variant_name_char() {
        assert_eq!(variant_name(&RfcValue::Char("x".into())), "Char");
    }

    // ── integer narrowing ───────────────────────────────────────

    #[test]
    fn narrow_int_native_boundaries() {
        // Exact bounds of every width narrow successfully.
        assert!(matches!(
            narrow_int(0, RFCTYPE::RFCTYPE_INT1, "F", native()),
            Ok(NarrowedInt::I1(0))
        ));
        assert!(matches!(
            narrow_int(255, RFCTYPE::RFCTYPE_INT1, "F", native()),
            Ok(NarrowedInt::I1(255))
        ));
        assert!(matches!(
            narrow_int(-32768, RFCTYPE::RFCTYPE_INT2, "F", native()),
            Ok(NarrowedInt::I2(-32768))
        ));
        assert!(matches!(
            narrow_int(32767, RFCTYPE::RFCTYPE_INT2, "F", native()),
            Ok(NarrowedInt::I2(32767))
        ));
        assert!(matches!(
            narrow_int(i128::from(i32::MIN), RFCTYPE::RFCTYPE_INT, "F", native()),
            Ok(NarrowedInt::I4(i32::MIN))
        ));
        assert!(matches!(
            narrow_int(i128::from(i64::MAX), RFCTYPE::RFCTYPE_INT8, "F", native()),
            Ok(NarrowedInt::I8(i64::MAX))
        ));
    }

    #[test]
    fn narrow_int_native_range_errors() {
        // One past each bound errors with path, value and range.
        let err = narrow_int(256, RFCTYPE::RFCTYPE_INT1, "S.RFCINT1", native()).unwrap_err();
        assert!(matches!(err, RfcError::InvalidInput(_)));
        let msg = err.to_string();
        assert!(msg.contains("S.RFCINT1"), "{msg}");
        assert!(msg.contains("256"), "{msg}");
        assert!(msg.contains("INT1 (0..255)"), "{msg}");

        let err = narrow_int(-1, RFCTYPE::RFCTYPE_INT1, "F", native()).unwrap_err();
        assert!(err.to_string().contains("INT1 (0..255)"));

        let err = narrow_int(32768, RFCTYPE::RFCTYPE_INT2, "F", native()).unwrap_err();
        assert!(err.to_string().contains("INT2"));

        let err = narrow_int(
            i128::from(i32::MAX) + 1,
            RFCTYPE::RFCTYPE_INT,
            "F",
            native(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("INT4"));

        let err = narrow_int(
            i128::from(i64::MAX) + 1,
            RFCTYPE::RFCTYPE_INT8,
            "F",
            native(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("INT8"));
    }

    #[test]
    fn narrow_int_pyrfc_dispatch() {
        // pyrfc mode: INT1/INT2/INT all narrow to i32 (RfcSetInt), so
        // 256 → INT1 passes through for the SDK to judge.
        let compat = CompatOptions {
            pyrfc_int_setters: true,
            ..CompatOptions::default()
        };
        assert!(matches!(
            narrow_int(256, RFCTYPE::RFCTYPE_INT1, "F", compat),
            Ok(NarrowedInt::I4(256))
        ));
        assert!(matches!(
            narrow_int(40000, RFCTYPE::RFCTYPE_INT2, "F", compat),
            Ok(NarrowedInt::I4(40000))
        ));
        // Beyond i32 errors client-side (pyrfc raised OverflowError there).
        let err =
            narrow_int(i128::from(i32::MAX) + 1, RFCTYPE::RFCTYPE_INT1, "F", compat).unwrap_err();
        assert!(matches!(err, RfcError::InvalidInput(_)));
        // INT8 narrows to i64.
        assert!(matches!(
            narrow_int(i128::from(i64::MAX), RFCTYPE::RFCTYPE_INT8, "F", compat),
            Ok(NarrowedInt::I8(i64::MAX))
        ));
        let err =
            narrow_int(i128::from(i64::MAX) + 1, RFCTYPE::RFCTYPE_INT8, "F", compat).unwrap_err();
        assert!(matches!(err, RfcError::InvalidInput(_)));
    }

    #[test]
    fn int_value_extracts_all_integer_variants() {
        assert_eq!(int_value(&RfcValue::Int(1)), Some(1));
        assert_eq!(int_value(&RfcValue::Int1(2)), Some(2));
        assert_eq!(int_value(&RfcValue::Int2(-3)), Some(-3));
        assert_eq!(int_value(&RfcValue::Int4(4)), Some(4));
        assert_eq!(int_value(&RfcValue::Int8(-5)), Some(-5));
        assert_eq!(int_value(&RfcValue::String("6".into())), None);
        assert_eq!(int_value(&RfcValue::Float(7.0)), None);
        assert_eq!(int_value(&RfcValue::Decimal("8".into())), None);
    }

    #[test]
    fn int_length_checked_against_numc_not_char() {
        // A 5-digit int into NUMC(4) → width error (int is valid for NUMC).
        let v = RfcValue::Int(12345);
        let err = check_input_length(&v, RFCTYPE::RFCTYPE_NUM, 8, "F", native()).unwrap_err();
        assert!(matches!(err, RfcError::InvalidInput(_)));
        assert!(err.to_string().contains("NUMC(4)"));

        // The same int into CHAR(4) passes the length check — set_field
        // rejects it as a type mismatch instead (a width error would
        // mislead).
        assert!(check_input_length(&v, RFCTYPE::RFCTYPE_CHAR, 8, "F", native()).is_ok());
    }

    #[test]
    fn rfctype_name_covers_int_targets() {
        assert_eq!(rfctype_name(RFCTYPE::RFCTYPE_INT), "INT4");
        assert_eq!(rfctype_name(RFCTYPE::RFCTYPE_INT1), "INT1");
        assert_eq!(rfctype_name(RFCTYPE::RFCTYPE_CHAR), "CHAR");
    }

    // ── decimal input validation ────────────────────────────────

    #[test]
    fn is_valid_decimal_accepts_numbers() {
        for s in [
            "0",
            "12345.67",
            "-12.345",
            "+3.14",
            ".5",
            "5.",
            "1e10",
            "-2.5E-3",
            "1234567890123456789012345678901234", // 34 digits (DECF34)
            " 12.5 ",
            "\t-1\n",
            "0.0",
            "1E+5",
        ] {
            assert!(is_valid_decimal(s), "should accept {s:?}");
        }
    }

    #[test]
    fn is_valid_decimal_accepts_decimal_special_values() {
        // Python's Decimal grammar (pyrfc's validity notion) accepts these;
        // native emits Decimal('NaN') for empty reads, so round-trip
        // of that value must validate.
        for s in [
            "NaN", "nan", "-NaN", "sNaN", "Infinity", "inf", "-Inf", "NaN123",
        ] {
            assert!(is_valid_decimal(s), "should accept {s:?}");
        }
    }

    #[test]
    fn is_valid_decimal_rejects_garbage() {
        // The bug this guards: RfcSetString turns these into wrong numbers.
        for s in [
            "", " ", "A", "x9y", "1.2.3", "1e", "e5", "--1", "1,5", "0x1f", "1 2", "12a", "nana",
            "in",
        ] {
            assert!(!is_valid_decimal(s), "should reject {s:?}");
        }
    }
}
