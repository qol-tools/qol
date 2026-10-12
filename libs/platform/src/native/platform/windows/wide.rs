use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Globalization::MultiByteToWideChar;

pub use crate::utf16::from_wide;

pub fn wide_nul(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

pub fn from_code_page(bytes: &[u8], code_page: u32) -> Option<String> {
    let len = i32::try_from(bytes.len()).ok()?;
    let wide_len =
        unsafe { MultiByteToWideChar(code_page, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
    if wide_len <= 0 {
        return None;
    }
    let mut wide = vec![0u16; wide_len as usize];
    let written = unsafe {
        MultiByteToWideChar(
            code_page,
            0,
            bytes.as_ptr(),
            len,
            wide.as_mut_ptr(),
            wide_len,
        )
    };
    if written <= 0 {
        return None;
    }
    wide.truncate(written as usize);
    Some(String::from_utf16_lossy(&wide))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_strings_are_nul_terminated() {
        let cases: [(&str, &[u16]); 3] = [
            ("", &[0]),
            ("ab", &[0x61, 0x62, 0]),
            ("\u{1F600}", &[0xD83D, 0xDE00, 0]),
        ];
        for (text, expected) in cases {
            assert_eq!(wide_nul(text), expected, "{text:?}");
        }
    }

    #[test]
    fn code_page_bytes_decode_through_the_given_code_page() {
        let cases: [(&[u8], u32, &str); 2] =
            [(&[0x81, 0x94], 850, "üö"), (&[b'a', 0x80], 1252, "a€")];
        for (bytes, code_page, expected) in cases {
            assert_eq!(
                from_code_page(bytes, code_page).as_deref(),
                Some(expected),
                "{code_page}"
            );
        }
    }

    #[test]
    fn wide_round_trips_through_from_wide() {
        for text in ["", "QoL Tray", r"C:\Users\x y\qol-tray.exe", "\u{1F600}"] {
            assert_eq!(from_wide(&wide_nul(text)), text);
        }
    }
}
