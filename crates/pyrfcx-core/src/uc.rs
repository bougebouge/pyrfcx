// str ⇄ SAP_UC (UTF-16) conversion — the single centralized module (constraint #7).
//
// No other module touches UTF-16 encoding directly. All functions are
// pub(crate) — external consumers go through higher-level APIs.

use crate::error::RfcError;

/// Encode text that the SDK reads as a NUL-terminated string: a name or a
/// setting (connection parameter name and value, function, parameter and
/// field name, repository id, path).
///
/// The rule: an embedded NUL fails with [`RfcError::InvalidInput`]. The
/// SDK stops reading at the first NUL, so it would otherwise get a shorter
/// name or setting without any error. The check is part of the conversion,
/// so it runs before the SDK call that takes the text.
///
/// `what` names the input in the message (`embedded null character in
/// {what}`). It runs only on failure. The message never contains the text
/// itself: a connection parameter value can be a password.
pub(crate) fn encode_nul_terminated(
    s: &str,
    what: impl FnOnce() -> String,
) -> Result<Vec<u16>, RfcError> {
    if s.contains('\0') {
        return Err(RfcError::InvalidInput(format!(
            "embedded null character in {}",
            what()
        )));
    }
    Ok(encode_unchecked(s))
}

/// Encode text with a NUL terminator appended and NO check for an embedded
/// NUL.
///
/// Only for a value that the SDK takes with an explicit length
/// (`RfcSetString`), where an embedded NUL reaches the SDK, and for raw
/// SDK probes in tests and tools. A name or a setting goes through
/// [`encode_nul_terminated`].
#[doc(hidden)]
pub fn encode_unchecked(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Encode a Rust string to a UTF-16 buffer of exactly `len` code units,
/// padded with blanks (0x0020). No null terminator. Used for CHAR/NUMC
/// fields where the SDK expects a fixed-width buffer.
///
/// # Panics
///
/// Panics if the encoded string is longer than `len` code units.
#[allow(dead_code)]
pub(crate) fn encode_padded(s: &str, len: usize) -> Vec<u16> {
    let mut buf: Vec<u16> = s.encode_utf16().collect();
    assert!(
        buf.len() <= len,
        "string encodes to {} UTF-16 code units, exceeds buffer length {}",
        buf.len(),
        len,
    );
    buf.resize(len, 0x0020);
    buf
}

/// Decode a UTF-16 slice to a Rust `String`.
/// Stops at the first null (0x0000), if present.
/// Uses `from_utf16_lossy` as a defensive measure at the FFI boundary.
pub(crate) fn decode(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Decode a UTF-16 slice of exactly `len` code units (no null scanning).
/// Used when the length is known from the SDK (e.g., `RfcGetStringLength`).
pub(crate) fn decode_len(buf: &[u16], len: usize) -> String {
    String::from_utf16_lossy(&buf[..len])
}

/// Decode a UTF-16 slice representing a NUMC digit buffer (the SDK type `RFC_NUM`).
/// Functionally identical to [`decode`] — separated for semantic clarity.
pub(crate) fn decode_numc(buf: &[u16]) -> String {
    decode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── encode ──────────────────────────────────────────────────────

    #[test]
    fn encode_ascii() {
        assert_eq!(
            encode_unchecked("HELLO"),
            [0x48, 0x45, 0x4C, 0x4C, 0x4F, 0x00]
        );
    }

    #[test]
    fn encode_empty() {
        assert_eq!(encode_unchecked(""), [0x00]);
    }

    #[test]
    fn encode_bmp_non_ascii() {
        assert_eq!(encode_unchecked("Ü"), [0x00DC, 0x00]);
    }

    #[test]
    fn encode_surrogate_pair() {
        // 𝄞 = U+1D11E → surrogate pair D834 DD1E
        assert_eq!(encode_unchecked("𝄞"), [0xD834, 0xDD1E, 0x00]);
    }

    #[test]
    fn encode_cjk_extension_b() {
        // 𠀀 = U+20000 → surrogate pair D840 DC00
        assert_eq!(encode_unchecked("𠀀"), [0xD840, 0xDC00, 0x00]);
    }

    #[test]
    fn encode_embedded_spaces() {
        assert_eq!(encode_unchecked("A B"), [0x41, 0x20, 0x42, 0x00]);
    }

    // ── encode_nul_terminated ───────────────────────────────────────

    #[test]
    fn encode_nul_terminated_rejects_an_embedded_nul_and_never_prints_the_text() {
        assert_eq!(
            encode_nul_terminated("STFC_CONNECTION", || unreachable!()).unwrap(),
            encode_unchecked("STFC_CONNECTION")
        );
        assert_eq!(encode_nul_terminated("", || unreachable!()).unwrap(), [0]);
        for text in ["se\0cret", "\0", "secret\0"] {
            match encode_nul_terminated(text, || "connection parameter 'passwd'".into()) {
                Err(RfcError::InvalidInput(msg)) => assert_eq!(
                    msg,
                    "embedded null character in connection parameter 'passwd'"
                ),
                other => panic!("expected InvalidInput for {text:?}, got {other:?}"),
            }
        }
    }

    // ── encode_padded ───────────────────────────────────────────────

    #[test]
    fn encode_padded_short() {
        assert_eq!(encode_padded("AB", 5), [0x41, 0x42, 0x20, 0x20, 0x20]);
    }

    #[test]
    fn encode_padded_exact() {
        assert_eq!(encode_padded("ABC", 3), [0x41, 0x42, 0x43]);
    }

    #[test]
    #[should_panic(expected = "exceeds buffer length")]
    fn encode_padded_too_long() {
        encode_padded("ABCDEF", 3);
    }

    #[test]
    fn encode_padded_empty() {
        assert_eq!(encode_padded("", 3), [0x20, 0x20, 0x20]);
    }

    #[test]
    fn encode_padded_zero_len_empty() {
        assert_eq!(encode_padded("", 0), &[] as &[u16]);
    }

    #[test]
    fn encode_padded_surrogate_pair_exact() {
        // 𝄞 = 2 code units, buffer len 2 → exact fit
        assert_eq!(encode_padded("𝄞", 2), [0xD834, 0xDD1E]);
    }

    #[test]
    fn encode_padded_surrogate_pair_with_padding() {
        // 𝄞 = 2 code units, buffer len 4 → 2 blanks
        assert_eq!(encode_padded("𝄞", 4), [0xD834, 0xDD1E, 0x20, 0x20]);
    }

    #[test]
    #[should_panic(expected = "exceeds buffer length")]
    fn encode_padded_surrogate_straddle() {
        // 𝄞 = 2 code units, buffer len 1 → cannot fit surrogate pair
        encode_padded("𝄞", 1);
    }

    #[test]
    #[should_panic(expected = "exceeds buffer length")]
    fn encode_padded_ascii_plus_surrogate_straddle() {
        // "A𝄞" = 3 code units (1 + 2), buffer len 2 → too long
        encode_padded("A𝄞", 2);
    }

    // ── decode ──────────────────────────────────────────────────────

    #[test]
    fn decode_with_trailing_null() {
        assert_eq!(decode(&[0x41, 0x00, 0x42]), "A");
    }

    #[test]
    fn decode_no_null() {
        assert_eq!(decode(&[0x41, 0x42]), "AB");
    }

    #[test]
    fn decode_all_null() {
        assert_eq!(decode(&[0x00, 0x00]), "");
    }

    #[test]
    fn decode_empty_slice() {
        assert_eq!(decode(&[]), "");
    }

    #[test]
    fn decode_surrogate_pair() {
        assert_eq!(decode(&[0xD834, 0xDD1E, 0x00]), "𝄞");
    }

    #[test]
    fn decode_bmp_non_ascii() {
        assert_eq!(decode(&[0x00DC, 0x00]), "Ü");
    }

    // ── decode_len ──────────────────────────────────────────────────

    #[test]
    fn decode_len_partial() {
        assert_eq!(decode_len(&[0x41, 0x00, 0x42], 3), "A\0B");
    }

    #[test]
    fn decode_len_zero() {
        assert_eq!(decode_len(&[0x41, 0x42], 0), "");
    }

    // ── decode_numc ─────────────────────────────────────────────────

    #[test]
    fn decode_numc_leading_zeros() {
        // "046\0" as UTF-16
        assert_eq!(decode_numc(&[0x30, 0x34, 0x36, 0x00]), "046");
    }

    #[test]
    fn decode_numc_all_digits() {
        assert_eq!(decode_numc(&[0x31, 0x32, 0x33]), "123");
    }

    // ── round-trip ──────────────────────────────────────────────────

    #[test]
    fn round_trip() {
        let cases = [
            "",
            "HELLO",
            "Ü",
            "𝄞",
            "𠀀",
            "A B",
            "Hello, 世界!",
            "abc𝄞def",
        ];
        for s in cases {
            let encoded = encode_unchecked(s);
            let decoded = decode(&encoded);
            assert_eq!(decoded, s, "round-trip failed for {s:?}");
        }
    }
}
