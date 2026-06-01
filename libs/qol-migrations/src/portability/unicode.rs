//! Unicode normalization helpers for cross-OS profile/file names.
//!
//! macOS (HFS+/APFS) historically stores filenames in NFD (decomposed) form,
//! while Linux ext4 and Windows NTFS preserve the bytes written, which in
//! practice means NFC. A profile named `café` written on macOS therefore
//! does not byte-match the same name written on Linux unless both sides
//! normalize to a single canonical form before comparing or persisting.
//!
//! NFC is chosen as the canonical form: it is the most compact, the most
//! common on the wire, and the form most cross-platform tooling expects.

use unicode_normalization::UnicodeNormalization;

/// Normalize a string to Unicode NFC (Normalization Form Canonical Composition).
///
/// All profile names, file names, and any other cross-OS identifier must be
/// passed through this before being compared, stored, or written to disk.
pub fn normalize_to_nfc(s: &str) -> String {
    s.nfc().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfd_decomposed_cafe_becomes_nfc_precomposed() {
        let nfd = "cafe\u{0301}";
        let nfc = "caf\u{00e9}";
        assert_eq!(normalize_to_nfc(nfd), nfc, "input: {nfd:?}");
        assert_ne!(nfd, nfc, "sanity: NFD and NFC differ byte-wise");
    }

    #[test]
    fn already_nfc_strings_pass_through_unchanged() {
        let cases = [
            "default",
            "café",
            "",
            "ascii-only",
            "日本語",
            "mixed-café-日本",
        ];
        for input in cases {
            assert_eq!(normalize_to_nfc(input), input, "input: {input:?}");
        }
    }
}
