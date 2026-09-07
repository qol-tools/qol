const GB: f64 = 1_000_000_000.0;
const MB: f64 = 1_000_000.0;
const KB: f64 = 1_000.0;

pub fn format_bytes(bytes: u64) -> String {
    let gb = bytes as f64 / GB;
    let mb = bytes as f64 / MB;
    let kb = bytes as f64 / KB;
    if gb >= 1.0 || mb >= 999.95 {
        format!("{gb:.1} GB")
    } else if mb >= 1.0 || kb >= 999.5 {
        format!("{mb:.1} MB")
    } else if kb >= 1.0 {
        format!("{kb:.0} KB")
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn format_bytes_uses_decimal_units() {
        let cases = [
            (0, "0 B"),
            (999, "999 B"),
            (1_000, "1 KB"),
            (999_499, "999 KB"),
            (999_999, "1.0 MB"),
            (1_000_000, "1.0 MB"),
            (1_500_000, "1.5 MB"),
            (999_949_999, "999.9 MB"),
            (999_999_999, "1.0 GB"),
            (1_000_000_000, "1.0 GB"),
            (2_500_000_000, "2.5 GB"),
        ];
        for (bytes, expected) in cases {
            assert_eq!(format_bytes(bytes), expected, "bytes: {bytes}");
        }
    }
}
