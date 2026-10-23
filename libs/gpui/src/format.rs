const STEP: f64 = 1_000.0;
const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1_000 {
        return format!("{bytes} B");
    }
    let mut amount = bytes as f64 / STEP;
    let mut unit = 0;
    while unit + 1 < UNITS.len() && amount.round() >= STEP {
        amount /= STEP;
        unit += 1;
    }
    let tenths = (amount * 10.0).round() as u64;
    if tenths < 100 {
        format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[unit])
    } else {
        format!("{} {}", amount.round() as u64, UNITS[unit])
    }
}

pub use qol_config::text::plural;

#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn format_bytes_uses_decimal_units() {
        let cases = [
            (0, "0 B"),
            (999, "999 B"),
            (1_000, "1.0 KB"),
            (999_499, "999 KB"),
            (999_500, "1.0 MB"),
            (1_000_000, "1.0 MB"),
            (1_500_000, "1.5 MB"),
            (999_499_999, "999 MB"),
            (999_949_999, "1.0 GB"),
            (1_000_000_000, "1.0 GB"),
            (2_500_000_000, "2.5 GB"),
            (999_999_999_999, "1.0 TB"),
            (3_298_534_883_328, "3.3 TB"),
            (140_737_488_355_328, "141 TB"),
            (2_000_000_000_000_000, "2000 TB"),
        ];
        for (bytes, expected) in cases {
            assert_eq!(format_bytes(bytes), expected, "bytes: {bytes}");
        }
    }

    #[test]
    fn format_bytes_shows_one_decimal_below_ten_and_whole_numbers_from_ten() {
        let cases = [
            (1_023, "1.0 KB"),
            (1_499, "1.5 KB"),
            (1_500, "1.5 KB"),
            (9_949, "9.9 KB"),
            (9_951, "10 KB"),
            (10_000, "10 KB"),
            (15_400_000, "15 MB"),
            (9_960_000_000, "10 GB"),
            (99_500_000_000, "100 GB"),
        ];
        for (bytes, expected) in cases {
            assert_eq!(format_bytes(bytes), expected, "bytes: {bytes}");
        }
    }
}
