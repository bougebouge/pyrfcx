// The pyrfcx value model — lossless representation of RFC field values.

use indexmap::IndexMap;

/// A single RFC field value, preserving full SDK fidelity.
///
/// Every ABAP/RFC data type maps to exactly one variant. The core never
/// loses precision or strips whitespace — lossy projections live only in
/// the compat output profile.
#[derive(Debug, Clone, PartialEq)]
pub enum RfcValue {
    /// RFCTYPE_CHAR — fixed-length, blank-padded text. Carries
    /// the FULL padded value (lossless); the value's UTF-16 length IS
    /// the field width, so no separate width field is needed. The core
    /// never strips — trailing pad blanks (U+0020) are removed only in
    /// the pyrfcx-py native output profile.
    Char(String),
    /// RFCTYPE_STRING — variable-length text. Trailing whitespace is
    /// representable data and is never stripped natively.
    String(String),
    /// NUM (NUMC) — leading-zero-padded digit string (ABAP naming).
    Numc(String),
    /// BCD, DECF16, DECF34 — SDK-produced decimal string.
    Decimal(String),
    /// FLOAT — IEEE 754 double.
    Float(f64),
    /// Width-agnostic integer input — the carrier for Python
    /// `int`, which is arbitrary-precision and has no inherent width.
    /// The marshaller range-checks against the target field's width and
    /// dispatches to the typed setter. i128 covers every representable
    /// integer target (INT8, NUMC/BCD up to 38 digits). Never produced
    /// on output — reads use the width-specific variants below.
    Int(i128),
    /// INT1 — unsigned 1-byte integer.
    Int1(u8),
    /// INT2 — signed 2-byte integer.
    Int2(i16),
    /// INT4 (RFCTYPE_INT) — signed 4-byte integer.
    Int4(i32),
    /// INT8 — signed 8-byte integer.
    Int8(i64),
    /// DATE — "YYYYMMDD" including initial "00000000".
    Date(String),
    /// TIME — "HHMMSS" including initial "000000".
    Time(String),
    /// BYTE, XSTRING — raw binary data.
    Bytes(Vec<u8>),
    /// UTCLONG, UTCSECOND, UTCMINUTE, DTDAY, DTWEEK, DTMONTH,
    /// TSECOND, TMINUTE, CDAY — new temporal types (32–40), read via
    /// `RfcGetString`.
    Temporal(String),
    /// ABAP structure — ordered field map (metadata index preserved).
    Structure(IndexMap<String, RfcValue>),
    /// ABAP internal table — vector of row maps.
    Table(Vec<IndexMap<String, RfcValue>>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_variant() {
        let v = RfcValue::Char("AB  ".into());
        assert!(matches!(v, RfcValue::Char(ref s) if s == "AB  "));
    }

    #[test]
    fn char_vs_string_identity() {
        let c = RfcValue::Char("AB".into());
        let s = RfcValue::String("AB".into());
        assert_ne!(c, s, "Char and String with same content must not be equal");
    }

    #[test]
    fn string_variant() {
        let v = RfcValue::String("HELLO".into());
        assert!(matches!(v, RfcValue::String(ref s) if s == "HELLO"));
    }

    #[test]
    fn numc_variant() {
        let v = RfcValue::Numc("0042".into());
        assert!(matches!(v, RfcValue::Numc(ref s) if s == "0042"));
    }

    #[test]
    fn decimal_variant() {
        let v = RfcValue::Decimal("123.45".into());
        assert!(matches!(v, RfcValue::Decimal(ref s) if s == "123.45"));
    }

    #[test]
    fn float_variant() {
        let v = RfcValue::Float(1.23);
        assert!(matches!(v, RfcValue::Float(f) if (f - 1.23).abs() < f64::EPSILON));
    }

    #[test]
    fn int_variants() {
        assert!(matches!(RfcValue::Int(i128::MAX), RfcValue::Int(i128::MAX)));
        assert!(matches!(RfcValue::Int1(255), RfcValue::Int1(255)));
        assert!(matches!(RfcValue::Int2(-1), RfcValue::Int2(-1)));
        assert!(matches!(RfcValue::Int4(42), RfcValue::Int4(42)));
        assert!(matches!(RfcValue::Int8(i64::MAX), RfcValue::Int8(i64::MAX)));
    }

    #[test]
    fn int_vs_int4_identity() {
        // Width-agnostic input carrier and width-specific output variant
        // with the same value must not compare equal.
        assert_ne!(RfcValue::Int(42), RfcValue::Int4(42));
    }

    #[test]
    fn date_variant() {
        let v = RfcValue::Date("20260714".into());
        assert!(matches!(v, RfcValue::Date(ref s) if s == "20260714"));
    }

    #[test]
    fn time_variant() {
        let v = RfcValue::Time("120000".into());
        assert!(matches!(v, RfcValue::Time(ref s) if s == "120000"));
    }

    #[test]
    fn bytes_variant() {
        let v = RfcValue::Bytes(vec![0xDE, 0xAD]);
        assert!(matches!(v, RfcValue::Bytes(ref b) if b == &[0xDE, 0xAD]));
    }

    #[test]
    fn temporal_variant() {
        let v = RfcValue::Temporal("2026-07-14T12:00:00.0000000".into());
        assert!(matches!(v, RfcValue::Temporal(_)));
    }

    #[test]
    fn structure_variant() {
        let mut m = IndexMap::new();
        m.insert("FIELD1".into(), RfcValue::String("val".into()));
        m.insert("FIELD2".into(), RfcValue::Int4(42));
        let v = RfcValue::Structure(m);
        if let RfcValue::Structure(ref map) = v {
            assert_eq!(map.len(), 2);
            assert_eq!(map.get_index(0).unwrap().0, "FIELD1");
        } else {
            panic!("expected Structure");
        }
    }

    #[test]
    fn table_variant() {
        let row = IndexMap::from([("COL".into(), RfcValue::Int4(1))]);
        let v = RfcValue::Table(vec![row]);
        if let RfcValue::Table(ref rows) = v {
            assert_eq!(rows.len(), 1);
        } else {
            panic!("expected Table");
        }
    }

    #[test]
    fn clone_and_eq() {
        let v = RfcValue::String("test".into());
        let v2 = v.clone();
        assert_eq!(v, v2);
    }
}
