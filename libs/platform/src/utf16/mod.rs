pub fn from_wide(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

pub fn from_le_bytes(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    from_wide(&units)
}

#[cfg(test)]
mod tests {
    use super::{from_le_bytes, from_wide};

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

    fn le_bytes(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn decodes_up_to_the_first_nul_unit_without_trimming() {
        let cases = [
            (le_bytes(" Luna 2 \0garbage"), " Luna 2 "),
            (le_bytes("no terminator"), "no terminator"),
            (le_bytes("\u{1F600}"), "\u{1F600}"),
            (le_bytes("\0"), ""),
            (Vec::new(), ""),
            ([le_bytes("ab"), vec![0x63]].concat(), "ab"),
            (vec![0x00, 0xD8, 0x61, 0x00], "\u{FFFD}a"),
            (vec![0x00, 0x01, 0x00, 0x00, 0x62, 0x00], "\u{100}"),
        ];
        for (bytes, expected) in cases {
            assert_eq!(from_le_bytes(&bytes), expected, "{bytes:?}");
        }
    }
}
