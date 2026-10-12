pub(super) fn decode(bytes: &[u8], code_page: impl FnOnce(&[u8]) -> Option<String>) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => code_page(bytes).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_utf8_output_is_kept_without_the_code_page() {
        let cases = ["", "plain", "ünicöde", "line\r\nnext"];
        for text in cases {
            let decoded = decode(text.as_bytes(), |_| panic!("code page used for {text:?}"));
            assert_eq!(decoded, text);
        }
    }

    #[test]
    fn output_that_is_not_utf8_decodes_with_the_code_page() {
        let decoded = decode(&[0x81, 0x94], |bytes| {
            assert_eq!(bytes, [0x81, 0x94]);
            Some("üö".to_string())
        });
        assert_eq!(decoded, "üö");
    }

    #[test]
    fn a_failed_code_page_decode_falls_back_to_replacement_characters() {
        assert_eq!(decode(&[b'a', 0x81], |_| None), "a\u{fffd}");
    }
}
