// Read-only metadata description of a function module's signature.
//
// The marshaller (`marshal.rs`) reads field types straight off the SDK
// descriptors as it walks a value tree. This module exposes the same
// metadata *ahead* of marshalling, as owned Rust data, so that callers
// which hold information the core cannot see — pyrfcx-py, which holds the
// caller's live Python objects — can make type-directed decisions before
// any value is converted.
//
// Nothing here is compat-specific: it is a faithful, lossless projection
// of the SDK's own `RFC_PARAMETER_DESC` / `RFC_FIELD_DESC` metadata.

use crate::error::RfcError;
use crate::ffi::bindings::RFC_ABAP_NAME;
use crate::ffi::{
    self, RFC_DIRECTION, RFC_FIELD_DESC, RFC_FUNCTION_DESC_HANDLE, RFC_PARAMETER_DESC, RFC_RC,
    RFC_TYPE_DESC_HANDLE, RFCTYPE,
};
use crate::uc;

/// The RFC type of a parameter or field — a stable, FFI-free mirror of the
/// SDK's `RFCTYPE` enum (7.50 header). `Other` carries the raw code for
/// types the header defines but we do not name individually, so callers can
/// still report it faithfully.
///
/// Note there is no `SSTRING`: no such constant exists in the 7.50 header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RfcFieldType {
    Char,
    Date,
    Bcd,
    Time,
    Byte,
    Table,
    /// NUMC — the SDK's `RFCTYPE_NUM` (ABAP naming over SDK naming).
    Numc,
    Float,
    Int,
    Int2,
    Int1,
    Null,
    AbapObject,
    Structure,
    Decf16,
    Decf34,
    XmlData,
    String,
    XString,
    Int8,
    UtcLong,
    UtcSecond,
    UtcMinute,
    DtDay,
    DtWeek,
    DtMonth,
    TSecond,
    TMinute,
    CDay,
    Box,
    GenericBox,
    /// A type code this enum does not name, carried verbatim.
    Other(i32),
}

impl RfcFieldType {
    /// The SDK's numeric type code.
    pub fn code(self) -> i32 {
        match self {
            Self::Char => 0,
            Self::Date => 1,
            Self::Bcd => 2,
            Self::Time => 3,
            Self::Byte => 4,
            Self::Table => 5,
            Self::Numc => 6,
            Self::Float => 7,
            Self::Int => 8,
            Self::Int2 => 9,
            Self::Int1 => 10,
            Self::Null => 14,
            Self::AbapObject => 16,
            Self::Structure => 17,
            Self::Decf16 => 23,
            Self::Decf34 => 24,
            Self::XmlData => 28,
            Self::String => 29,
            Self::XString => 30,
            Self::Int8 => 31,
            Self::UtcLong => 32,
            Self::UtcSecond => 33,
            Self::UtcMinute => 34,
            Self::DtDay => 35,
            Self::DtWeek => 36,
            Self::DtMonth => 37,
            Self::TSecond => 38,
            Self::TMinute => 39,
            Self::CDay => 40,
            Self::Box => 41,
            Self::GenericBox => 42,
            Self::Other(code) => code,
        }
    }

    // The bindgen enum newtypes wrap c_int on MSVC and c_uint elsewhere.
    #[allow(clippy::unnecessary_cast)]
    pub(crate) fn from_rfctype(t: RFCTYPE) -> Self {
        match t {
            RFCTYPE::RFCTYPE_CHAR => Self::Char,
            RFCTYPE::RFCTYPE_DATE => Self::Date,
            RFCTYPE::RFCTYPE_BCD => Self::Bcd,
            RFCTYPE::RFCTYPE_TIME => Self::Time,
            RFCTYPE::RFCTYPE_BYTE => Self::Byte,
            RFCTYPE::RFCTYPE_TABLE => Self::Table,
            RFCTYPE::RFCTYPE_NUM => Self::Numc,
            RFCTYPE::RFCTYPE_FLOAT => Self::Float,
            RFCTYPE::RFCTYPE_INT => Self::Int,
            RFCTYPE::RFCTYPE_INT2 => Self::Int2,
            RFCTYPE::RFCTYPE_INT1 => Self::Int1,
            RFCTYPE::RFCTYPE_NULL => Self::Null,
            RFCTYPE::RFCTYPE_ABAPOBJECT => Self::AbapObject,
            RFCTYPE::RFCTYPE_STRUCTURE => Self::Structure,
            RFCTYPE::RFCTYPE_DECF16 => Self::Decf16,
            RFCTYPE::RFCTYPE_DECF34 => Self::Decf34,
            RFCTYPE::RFCTYPE_XMLDATA => Self::XmlData,
            RFCTYPE::RFCTYPE_STRING => Self::String,
            RFCTYPE::RFCTYPE_XSTRING => Self::XString,
            RFCTYPE::RFCTYPE_INT8 => Self::Int8,
            RFCTYPE::RFCTYPE_UTCLONG => Self::UtcLong,
            RFCTYPE::RFCTYPE_UTCSECOND => Self::UtcSecond,
            RFCTYPE::RFCTYPE_UTCMINUTE => Self::UtcMinute,
            RFCTYPE::RFCTYPE_DTDAY => Self::DtDay,
            RFCTYPE::RFCTYPE_DTWEEK => Self::DtWeek,
            RFCTYPE::RFCTYPE_DTMONTH => Self::DtMonth,
            RFCTYPE::RFCTYPE_TSECOND => Self::TSecond,
            RFCTYPE::RFCTYPE_TMINUTE => Self::TMinute,
            RFCTYPE::RFCTYPE_CDAY => Self::CDay,
            RFCTYPE::RFCTYPE_BOX => Self::Box,
            RFCTYPE::RFCTYPE_GENERIC_BOX => Self::GenericBox,
            other => Self::Other(other.0 as i32),
        }
    }
}

/// Parameter direction, mirroring the SDK's `RFC_DIRECTION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RfcDirection {
    Import,
    Export,
    Changing,
    Tables,
    Other(i32),
}

impl RfcDirection {
    /// The SDK's numeric direction code.
    pub fn code(self) -> i32 {
        match self {
            Self::Import => 1,
            Self::Export => 2,
            Self::Changing => 3,
            Self::Tables => 7,
            Self::Other(code) => code,
        }
    }

    // The bindgen enum newtypes wrap c_int on MSVC and c_uint elsewhere.
    #[allow(clippy::unnecessary_cast)]
    pub(crate) fn from_rfc_direction(d: RFC_DIRECTION) -> Self {
        match d {
            RFC_DIRECTION::RFC_IMPORT => Self::Import,
            RFC_DIRECTION::RFC_EXPORT => Self::Export,
            RFC_DIRECTION::RFC_CHANGING => Self::Changing,
            RFC_DIRECTION::RFC_TABLES => Self::Tables,
            other => Self::Other(other.0 as i32),
        }
    }
}

/// Name and byte lengths of a structure or table line type, as the SDK's
/// type descriptor reports them (`RfcGetTypeName`, `RfcGetTypeLength`).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeInfo {
    /// The DDIC name of the type.
    pub name: String,
    /// Length in bytes in a 1-byte-per-character system.
    pub nuc_length: u32,
    /// Length in bytes in a 2-byte-per-character (Unicode) system.
    pub uc_length: u32,
}

/// The type of a parameter or field, plus the nested field list for
/// STRUCTURE / TABLE targets (empty for scalars).
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDesc {
    pub rfctype: RfcFieldType,
    /// Length in bytes in a 1-byte-per-character system.
    pub nuc_length: u32,
    /// Length in bytes of the Unicode (UTF-16) representation. For CHAR
    /// and NUMC the field width in characters is `uc_length / 2`.
    pub uc_length: u32,
    pub decimals: u32,
    /// The type descriptor's name and lengths, when the SDK attaches one
    /// (STRUCTURE / TABLE); `None` for scalar types.
    pub type_info: Option<TypeInfo>,
    /// Nested fields for STRUCTURE / TABLE; empty for scalar types.
    pub fields: Vec<NamedFieldDesc>,
}

impl FieldDesc {
    /// The nested field with this exact name, if any.
    pub fn field(&self, name: &str) -> Option<&NamedFieldDesc> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// A nested field: its DDIC name and offsets plus its description.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedFieldDesc {
    /// The canonical field name as the SDK reports it (upper-case).
    pub name: String,
    /// Offset in bytes in a 1-byte-per-character system.
    pub nuc_offset: u32,
    /// Offset in bytes in a 2-byte-per-character (Unicode) system.
    pub uc_offset: u32,
    pub desc: FieldDesc,
}

/// A function-module parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDesc {
    /// The canonical parameter name as the SDK reports it (upper-case).
    pub name: String,
    pub direction: RfcDirection,
    /// The default value as defined in SE37, verbatim.
    pub default_value: String,
    /// The parameter's description text as defined in SE37, verbatim.
    pub text: String,
    /// Whether SE37 defines the parameter as optional.
    pub optional: bool,
    pub desc: FieldDesc,
}

/// A function module's full signature.
#[derive(Debug, Clone, PartialEq)]
pub struct FuncDesc {
    /// The function module name as the SDK's descriptor reports it.
    pub name: String,
    pub params: Vec<ParamDesc>,
}

impl FuncDesc {
    /// The parameter with this exact name, if any.
    ///
    /// Matching is exact and case-sensitive, mirroring the SDK's own
    /// `RfcGetParameterDescByName` lookup that `marshal::set_inputs` uses.
    /// A caller-supplied name that misses here is not an error for this
    /// module to report — the marshaller will surface the SDK's own
    /// "parameter not found" error when the call proceeds.
    pub fn param(&self, name: &str) -> Option<&ParamDesc> {
        self.params.iter().find(|p| p.name == name)
    }
}

/// Build the owned description tree for a function descriptor handle.
pub(crate) fn describe(
    sdk: &ffi::SapNwRfc,
    func_desc: RFC_FUNCTION_DESC_HANDLE,
) -> Result<FuncDesc, RfcError> {
    let mut name_buf: RFC_ABAP_NAME = [0; 31];
    let mut error_info = ffi::zero_error_info();

    // Safety: func_desc is a valid function descriptor handle from the SDK
    // cache; name_buf is an RFC_ABAP_NAME buffer, the size the SDK expects.
    let rc = unsafe { sdk.RfcGetFunctionName(func_desc, name_buf.as_mut_ptr(), &mut error_info) };
    if rc != RFC_RC::RFC_OK {
        return Err(RfcError::from_error_info(ffi::extract_error_info(
            "RfcGetFunctionName",
            &error_info,
        )));
    }

    let mut count: std::ffi::c_uint = 0;
    let mut error_info = ffi::zero_error_info();

    // Safety: func_desc is a valid function descriptor handle from the SDK
    // cache; count is a valid out-param.
    let rc = unsafe { sdk.RfcGetParameterCount(func_desc, &mut count, &mut error_info) };
    if rc != RFC_RC::RFC_OK {
        return Err(RfcError::from_error_info(ffi::extract_error_info(
            "RfcGetParameterCount",
            &error_info,
        )));
    }

    let mut params = Vec::with_capacity(count as usize);
    for i in 0..count {
        let mut param_desc: RFC_PARAMETER_DESC = unsafe { std::mem::zeroed() };
        let mut error_info = ffi::zero_error_info();

        // Safety: func_desc is valid, i is in range, param_desc is a valid
        // out-param.
        let rc = unsafe {
            sdk.RfcGetParameterDescByIndex(func_desc, i, &mut param_desc, &mut error_info)
        };
        if rc != RFC_RC::RFC_OK {
            return Err(RfcError::from_error_info(ffi::extract_error_info(
                "RfcGetParameterDescByIndex",
                &error_info,
            )));
        }

        params.push(ParamDesc {
            name: uc::decode(&param_desc.name),
            direction: RfcDirection::from_rfc_direction(param_desc.direction),
            default_value: uc::decode(&param_desc.defaultValue),
            text: uc::decode(&param_desc.parameterText),
            optional: param_desc.optional != 0,
            desc: describe_field(
                sdk,
                param_desc.type_,
                param_desc.nucLength,
                param_desc.ucLength,
                param_desc.decimals,
                param_desc.typeDescHandle,
            )?,
        });
    }

    Ok(FuncDesc {
        name: uc::decode(&name_buf),
        params,
    })
}

/// Describe one field, recursing into the type descriptor the SDK attaches
/// to STRUCTURE / TABLE types.
///
/// Recursion terminates because DDIC types are acyclic: a structure cannot
/// contain itself, directly or transitively.
fn describe_field(
    sdk: &ffi::SapNwRfc,
    rfctype: RFCTYPE,
    nuc_length: u32,
    uc_length: u32,
    decimals: u32,
    type_desc: RFC_TYPE_DESC_HANDLE,
) -> Result<FieldDesc, RfcError> {
    let (type_info, fields) = if type_desc.is_null() {
        (None, Vec::new())
    } else {
        (
            Some(describe_type(sdk, type_desc)?),
            describe_fields(sdk, type_desc)?,
        )
    };

    Ok(FieldDesc {
        rfctype: RfcFieldType::from_rfctype(rfctype),
        nuc_length,
        uc_length,
        decimals,
        type_info,
        fields,
    })
}

/// Read a type descriptor's name and byte lengths.
fn describe_type(
    sdk: &ffi::SapNwRfc,
    type_desc: RFC_TYPE_DESC_HANDLE,
) -> Result<TypeInfo, RfcError> {
    let mut name_buf: RFC_ABAP_NAME = [0; 31];
    let mut error_info = ffi::zero_error_info();

    // Safety: type_desc is a non-null type descriptor handle from the SDK
    // cache; name_buf is an RFC_ABAP_NAME buffer, the size the SDK expects.
    let rc = unsafe { sdk.RfcGetTypeName(type_desc, name_buf.as_mut_ptr(), &mut error_info) };
    if rc != RFC_RC::RFC_OK {
        return Err(RfcError::from_error_info(ffi::extract_error_info(
            "RfcGetTypeName",
            &error_info,
        )));
    }

    let mut nuc_length: std::ffi::c_uint = 0;
    let mut uc_length: std::ffi::c_uint = 0;
    let mut error_info = ffi::zero_error_info();

    // Safety: type_desc is a non-null type descriptor handle; both lengths
    // are valid out-params.
    let rc = unsafe {
        sdk.RfcGetTypeLength(type_desc, &mut nuc_length, &mut uc_length, &mut error_info)
    };
    if rc != RFC_RC::RFC_OK {
        return Err(RfcError::from_error_info(ffi::extract_error_info(
            "RfcGetTypeLength",
            &error_info,
        )));
    }

    Ok(TypeInfo {
        name: uc::decode(&name_buf),
        nuc_length,
        uc_length,
    })
}

/// Describe every field of a structure/table line type.
fn describe_fields(
    sdk: &ffi::SapNwRfc,
    type_desc: RFC_TYPE_DESC_HANDLE,
) -> Result<Vec<NamedFieldDesc>, RfcError> {
    let mut count: std::ffi::c_uint = 0;
    let mut error_info = ffi::zero_error_info();

    // Safety: type_desc is a non-null type descriptor handle from the SDK
    // cache; count is a valid out-param.
    let rc = unsafe { sdk.RfcGetFieldCount(type_desc, &mut count, &mut error_info) };
    if rc != RFC_RC::RFC_OK {
        return Err(RfcError::from_error_info(ffi::extract_error_info(
            "RfcGetFieldCount",
            &error_info,
        )));
    }

    let mut fields = Vec::with_capacity(count as usize);
    for i in 0..count {
        let mut field_desc: RFC_FIELD_DESC = unsafe { std::mem::zeroed() };
        let mut error_info = ffi::zero_error_info();

        // Safety: type_desc is valid, i is in range, field_desc is a valid
        // out-param.
        let rc =
            unsafe { sdk.RfcGetFieldDescByIndex(type_desc, i, &mut field_desc, &mut error_info) };
        if rc != RFC_RC::RFC_OK {
            return Err(RfcError::from_error_info(ffi::extract_error_info(
                "RfcGetFieldDescByIndex",
                &error_info,
            )));
        }

        fields.push(NamedFieldDesc {
            name: uc::decode(&field_desc.name),
            nuc_offset: field_desc.nucOffset,
            uc_offset: field_desc.ucOffset,
            desc: describe_field(
                sdk,
                field_desc.type_,
                field_desc.nucLength,
                field_desc.ucLength,
                field_desc.decimals,
                field_desc.typeDescHandle,
            )?,
        });
    }

    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_type_codes_match_the_750_header() {
        // Spot-check the codes that the input dispatch keys on.
        assert_eq!(RfcFieldType::Char.code(), 0);
        assert_eq!(RfcFieldType::Date.code(), 1);
        assert_eq!(RfcFieldType::Bcd.code(), 2);
        assert_eq!(RfcFieldType::Time.code(), 3);
        assert_eq!(RfcFieldType::Byte.code(), 4);
        assert_eq!(RfcFieldType::Table.code(), 5);
        assert_eq!(RfcFieldType::Numc.code(), 6);
        assert_eq!(RfcFieldType::Float.code(), 7);
        assert_eq!(RfcFieldType::Int.code(), 8);
        assert_eq!(RfcFieldType::Int2.code(), 9);
        assert_eq!(RfcFieldType::Int1.code(), 10);
        assert_eq!(RfcFieldType::Structure.code(), 17);
        assert_eq!(RfcFieldType::Decf16.code(), 23);
        assert_eq!(RfcFieldType::Decf34.code(), 24);
        assert_eq!(RfcFieldType::String.code(), 29);
        assert_eq!(RfcFieldType::XString.code(), 30);
        assert_eq!(RfcFieldType::Int8.code(), 31);
        assert_eq!(RfcFieldType::UtcLong.code(), 32);
    }

    #[test]
    fn field_type_roundtrips_through_rfctype() {
        for ty in [
            RfcFieldType::Char,
            RfcFieldType::Date,
            RfcFieldType::Bcd,
            RfcFieldType::Time,
            RfcFieldType::Byte,
            RfcFieldType::Table,
            RfcFieldType::Numc,
            RfcFieldType::Float,
            RfcFieldType::Int,
            RfcFieldType::Int2,
            RfcFieldType::Int1,
            RfcFieldType::Null,
            RfcFieldType::AbapObject,
            RfcFieldType::Structure,
            RfcFieldType::Decf16,
            RfcFieldType::Decf34,
            RfcFieldType::XmlData,
            RfcFieldType::String,
            RfcFieldType::XString,
            RfcFieldType::Int8,
            RfcFieldType::UtcLong,
            RfcFieldType::UtcSecond,
            RfcFieldType::UtcMinute,
            RfcFieldType::DtDay,
            RfcFieldType::DtWeek,
            RfcFieldType::DtMonth,
            RfcFieldType::TSecond,
            RfcFieldType::TMinute,
            RfcFieldType::CDay,
            RfcFieldType::Box,
            RfcFieldType::GenericBox,
        ] {
            let code = ty.code();
            assert_eq!(
                RfcFieldType::from_rfctype(RFCTYPE(code as _)),
                ty,
                "type {ty:?} (code {code}) did not round-trip"
            );
        }
    }

    #[test]
    fn direction_roundtrips_through_rfc_direction() {
        for (dir, code) in [
            (RfcDirection::Import, 1),
            (RfcDirection::Export, 2),
            (RfcDirection::Changing, 3),
            (RfcDirection::Tables, 7),
            (RfcDirection::Other(5), 5),
        ] {
            assert_eq!(dir.code(), code);
            assert_eq!(
                RfcDirection::from_rfc_direction(RFC_DIRECTION(code as _)),
                dir
            );
        }
    }

    #[test]
    fn unknown_type_code_is_carried_verbatim() {
        // 13 is unassigned in the 7.50 header.
        let ty = RfcFieldType::from_rfctype(RFCTYPE(13));
        assert_eq!(ty, RfcFieldType::Other(13));
        assert_eq!(ty.code(), 13);
    }

    #[test]
    fn param_lookup_is_exact_and_case_sensitive() {
        let scalar = |ty| FieldDesc {
            rfctype: ty,
            nuc_length: 0,
            uc_length: 0,
            decimals: 0,
            type_info: None,
            fields: Vec::new(),
        };
        let desc = FuncDesc {
            name: "STFC_STRUCTURE".into(),
            params: vec![ParamDesc {
                name: "IMPORTSTRUCT".into(),
                direction: RfcDirection::Import,
                default_value: String::new(),
                text: String::new(),
                optional: false,
                desc: scalar(RfcFieldType::Structure),
            }],
        };

        assert!(desc.param("IMPORTSTRUCT").is_some());
        assert!(desc.param("importstruct").is_none());
        assert!(desc.param("NOPE").is_none());
    }

    #[test]
    fn nested_field_lookup() {
        let desc = FieldDesc {
            rfctype: RfcFieldType::Structure,
            nuc_length: 0,
            uc_length: 0,
            decimals: 0,
            type_info: None,
            fields: vec![NamedFieldDesc {
                name: "RFCINT1".into(),
                nuc_offset: 0,
                uc_offset: 0,
                desc: FieldDesc {
                    rfctype: RfcFieldType::Int1,
                    nuc_length: 1,
                    uc_length: 1,
                    decimals: 0,
                    type_info: None,
                    fields: Vec::new(),
                },
            }],
        };

        assert_eq!(
            desc.field("RFCINT1").map(|f| f.desc.rfctype),
            Some(RfcFieldType::Int1)
        );
        assert!(desc.field("rfcint1").is_none());
    }
}
