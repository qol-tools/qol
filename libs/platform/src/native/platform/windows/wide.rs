use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

pub fn wide_nul(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

pub fn from_wide(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
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
    fn from_wide_stops_at_the_first_nul() {
        let cases = [
            ("Luna 2\0garbage", "Luna 2"),
            ("no terminator", "no terminator"),
            ("\0", ""),
            ("", ""),
        ];
        for (raw, expected) in cases {
            let units: Vec<u16> = raw.encode_utf16().collect();
            assert_eq!(from_wide(&units), expected, "{raw:?}");
        }
    }

    #[test]
    fn wide_round_trips_through_from_wide() {
        for text in ["", "QoL Tray", r"C:\Users\x y\qol-tray.exe", "\u{1F600}"] {
            assert_eq!(from_wide(&wide_nul(text)), text);
        }
    }
}
