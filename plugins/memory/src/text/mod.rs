use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

fn token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[\p{L}\p{N}]+").expect("token regex"))
}

fn run_time_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"T([0-9]{2})-([0-9]{2})-([0-9]{2})-([0-9]{3})Z$").expect("run time regex")
    })
}

fn iso_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}):([0-9]{2})(?:\.([0-9]+))?(?:Z)$")
            .expect("iso regex")
    })
}

pub fn tokens(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    token_re()
        .find_iter(&lower)
        .filter(|m| m.as_str().chars().count() > 1)
        .map(|m| normalize(m.as_str()))
        .collect()
}

pub fn normalize(token: &str) -> String {
    let chars = token.chars().count();
    if chars <= 3 {
        return token.to_string();
    }
    if token.ends_with("ies") && chars > 4 {
        return format!("{}y", &token[..token.len() - 3]);
    }
    if token.ends_with("es")
        && chars > 4
        && (token.ends_with("ses")
            || token.ends_with("xes")
            || token.ends_with("zes")
            || token.ends_with("ches")
            || token.ends_with("shes"))
    {
        return token[..token.len() - 2].to_string();
    }
    if token.ends_with("ss") {
        return token.to_string();
    }
    if token.ends_with('s') && chars > 3 {
        return token[..token.len() - 1].to_string();
    }
    if token.ends_with("ing") && chars > 6 {
        return token[..token.len() - 3].to_string();
    }
    if token.ends_with("ed") && chars > 5 {
        return token[..token.len() - 2].to_string();
    }
    if token.ends_with("ly") && chars > 5 {
        return token[..token.len() - 2].to_string();
    }
    token.to_string()
}

pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

pub fn utf16_slice(text: &str, start: usize, end: usize) -> String {
    let len = text.encode_utf16().count();
    let start = start.min(len);
    let end = end.min(len);
    if start >= end {
        return String::new();
    }
    let units: Vec<u16> = text.encode_utf16().skip(start).take(end - start).collect();
    String::from_utf16_lossy(&units)
}

pub fn utf16_index_of(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    if nb.len() > hb.len() {
        return None;
    }
    for start in 0..=(hb.len() - nb.len()) {
        if &hb[start..start + nb.len()] == nb {
            return Some(haystack[..start].encode_utf16().count());
        }
    }
    None
}

pub(crate) fn is_js_space(ch: char) -> bool {
    ch.is_whitespace() || ch == '\u{feff}'
}

pub(crate) fn collapse_ws(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.chars() {
        if is_js_space(ch) {
            pending_space = true;
            continue;
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        out.push(ch);
        pending_space = false;
    }
    out
}

pub fn collapse_ws_lower(text: &str) -> String {
    collapse_ws(&text.to_lowercase())
}

pub fn token_jaccard(a: &str, b: &str) -> f64 {
    let token_set = |text: &str| -> HashSet<String> {
        collapse_ws_lower(text)
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let (left, right) = (token_set(a), token_set(b));
    let union = left.union(&right).count();
    if union == 0 {
        return 0.0;
    }
    left.intersection(&right).count() as f64 / union as f64
}

pub fn to_fixed2(value: f64) -> f64 {
    if !value.is_finite() || value.abs() >= 1e30 {
        return value;
    }
    let bits = value.to_bits();
    let negative = bits >> 63 == 1;
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (mantissa, exponent): (u128, i32) = if raw_exp == 0 {
        (frac as u128, -1074)
    } else {
        ((frac | (1u64 << 52)) as u128, raw_exp - 1075)
    };
    let cents = if exponent >= 0 {
        (mantissa * 100) << exponent
    } else {
        let scaled = mantissa * 100;
        let shift = (-exponent) as u32;
        if shift >= 128 {
            0
        } else {
            let q = scaled >> shift;
            let r = scaled - (q << shift);
            let bump = shift <= 127 && r * 2 >= 1u128 << shift;
            if bump {
                q + 1
            } else {
                q
            }
        }
    };
    let rounded = cents as f64 / 100.0;
    if negative {
        -rounded
    } else {
        rounded
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn parse_iso_millis(ts: Option<&str>) -> i64 {
    let Some(ts) = ts else {
        return 0;
    };
    let normalized = match ts.strip_suffix("+00:00") {
        Some(base) => format!("{base}Z"),
        None => ts.to_string(),
    };
    let Some(caps) = iso_re().captures(&normalized) else {
        return 0;
    };
    let num = |i: usize| -> i64 {
        caps.get(i)
            .and_then(|m| m.as_str().parse::<i64>().ok())
            .unwrap_or(0)
    };
    let (year, month, day, hour, minute, second) = (num(1), num(2), num(3), num(4), num(5), num(6));
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return 0;
    }
    let millis = caps
        .get(7)
        .map(|m| {
            m.as_str()
                .chars()
                .take(3)
                .fold(0, |acc, c| acc * 10 + c.to_digit(10).unwrap_or(0) as i64)
                * match m.as_str().len() {
                    1 => 100,
                    2 => 10,
                    _ => 1,
                }
        })
        .unwrap_or(0);
    let days = days_from_civil(year, month, day);
    days * 86_400_000 + (hour * 3600 + minute * 60 + second) * 1000 + millis
}

pub fn now_iso() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs() as i64;
    let millis = dur.subsec_millis();
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let sod = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

pub fn run_dir_millis(run: &str) -> i64 {
    parse_iso_millis(Some(
        run_time_re().replace(run, "T${1}:${2}:${3}.${4}Z").as_ref(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_port_table() {
        for (input, expected) in [
            ("houses", "hous"),
            ("hous", "hou"),
            ("processes", "process"),
            ("bodies", "body"),
            ("running", "runn"),
            ("wanted", "want"),
            ("quickly", "quick"),
            ("class", "class"),
            ("cats", "cat"),
        ] {
            assert_eq!(normalize(input), expected);
        }
    }

    #[test]
    fn tokens_punctuation_and_emoji() {
        assert_eq!(
            tokens("Cats RUNNIN,_processes!! \u{1f30d} café b2 a. Well-known: dogs."),
            vec!["cat", "runnin", "process", "café", "b2", "well", "known", "dog"]
        );
        assert_eq!(
            tokens("41 roads - Général Ünal码 3.14 one"),
            vec!["41", "road", "général", "ünal码", "14", "one"]
        );
        assert!(tokens("!!! ... 1").is_empty());
    }

    #[test]
    fn utf16_helpers_with_astral_char() {
        let s = "a\u{1f30d}bcé";
        assert_eq!(utf16_len(s), 6);
        assert_eq!(utf16_slice(s, 1, 3), "\u{1f30d}");
        assert_eq!(utf16_slice(s, 5, 99), "é");
        assert_eq!(utf16_slice(s, 4, 2), "");
        assert_eq!(utf16_index_of(s, "\u{1f30d}"), Some(1));
        assert_eq!(utf16_index_of(s, "cé"), Some(4));
        assert_eq!(utf16_index_of(s, "zz"), None);
        assert_eq!(utf16_index_of(s, ""), Some(0));
    }

    #[test]
    fn to_fixed2_js_half_up_table() {
        let cases: [(f64, f64); 10] = [
            (0.625, 0.63),
            (0.125, 0.13),
            (1.005, 1.0),
            (8.345, 8.35),
            (2.5, 2.5),
            (0.0, 0.0),
            (0.145, 0.14),
            (2.675, 2.67),
            (1.565, 1.56),
            (-8.345, -8.35),
        ];
        for (input, expected) in cases {
            assert_eq!(to_fixed2(input), expected);
        }
        assert_eq!(to_fixed2(f64::INFINITY), f64::INFINITY);
        assert_eq!(to_fixed2(f64::NEG_INFINITY), f64::NEG_INFINITY);
        assert!(to_fixed2(f64::NAN).is_nan());
    }

    #[test]
    fn parse_iso_node_values() {
        for (ts, ms) in [
            ("2026-08-27T08:39:05.554Z", 1787819945554),
            ("2026-08-27T08:39:05Z", 1787819945000),
            ("2026-08-27T08:39:05+00:00", 1787819945000),
            ("2026-01-01T00:00:00Z", 1767225600000),
            ("1969-07-20T20:17:40Z", -14182940000),
            ("2026-02-30T12:00:00Z", 1772452800000),
            ("2026-02-29T06:00:00Z", 1772344800000),
            ("2026-08-27T08:39:05.5Z", 1787819945500),
            ("2026-08-27T08:39:05.5549Z", 1787819945554),
            ("2026-08-27T08:39:05.55499999Z", 1787819945554),
        ] {
            assert_eq!(parse_iso_millis(Some(ts)), ms);
        }
        for bad in [
            "",
            "garbage",
            "2026-13-01T00:00:00Z",
            "2026-01-32T00:00:00Z",
            "2026-01-00T00:00:00Z",
            "2026-01-01T25:00:00Z",
            "2026-01-01T12:61:00Z",
            "2026-01-01T12:00:60Z",
            "2026-01-01 12:00:00Z",
        ] {
            assert_eq!(parse_iso_millis(Some(bad)), 0);
        }
        assert_eq!(parse_iso_millis(None), 0);
    }

    #[test]
    fn now_iso_shape_and_self_parse() {
        let stamp = now_iso();
        assert_eq!(stamp.len(), 24);
        assert!(stamp.ends_with('Z'));
        for i in 20..=22 {
            assert!(stamp.as_bytes()[i].is_ascii_digit());
        }
        for (i, expected) in [
            (4, '-'),
            (7, '-'),
            (10, 'T'),
            (13, ':'),
            (16, ':'),
            (19, '.'),
        ] {
            assert_eq!(stamp.as_bytes()[i], expected as u8);
        }
        let approx = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        assert!((approx - parse_iso_millis(Some(&stamp))).abs() < 2000);
    }

    #[test]
    fn collapse_ws_lower_rules() {
        assert_eq!(collapse_ws_lower("  A\t\tB\n\u{feff}C  "), "a b c");
        assert_eq!(collapse_ws_lower("Keep   the  Pace"), "keep the pace");
        assert_eq!(collapse_ws_lower("\t"), "");
        assert_eq!(collapse_ws_lower(""), "");
    }

    #[test]
    fn run_dir_time_conversion() {
        assert_eq!(run_dir_millis("2026-08-10T21-38-02-273Z"), 1786397882273);
        assert_eq!(run_dir_millis("not-a-run-dir"), 0);
    }
}
