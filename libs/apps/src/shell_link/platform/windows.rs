use qol_platform::native::wide::from_code_page;
use windows_sys::Win32::Globalization::CP_ACP;

pub(in crate::shell_link) fn decode_ansi(bytes: &[u8]) -> String {
    from_code_page(bytes, CP_ACP).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use qol_platform::native::wide::from_code_page;
    use windows_sys::Win32::Globalization::GetACP;

    #[test]
    fn a_non_unicode_link_string_decodes_through_the_ansi_code_page() {
        let expected = from_code_page(&[b'a', 0x80], unsafe { GetACP() }).unwrap();

        assert_eq!(crate::shell_link::ansi(&[b'a', 0x80, 0, b'z']), expected);
    }
}
