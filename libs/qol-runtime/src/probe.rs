#[cfg(debug_assertions)]
const LOG_FILE: &str = qol_conventions::TRACE_LOG_PATH;

#[cfg(debug_assertions)]
const MAX_LOG_BYTES: u64 = 16 * 1024 * 1024;

pub fn probe(tag: &str, msg: &str) {
    #[cfg(debug_assertions)]
    {
        use std::io::Write;

        rotate_if_needed();

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let line = format!("{ts} pid={} {tag} {msg}\n", std::process::id());
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(LOG_FILE)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (tag, msg);
    }
}

#[cfg(debug_assertions)]
fn rotate_if_needed() {
    let Ok(metadata) = std::fs::metadata(LOG_FILE) else {
        return;
    };
    if metadata.len() < MAX_LOG_BYTES {
        return;
    }

    let rotated = format!("{LOG_FILE}.1");
    let _ = std::fs::remove_file(&rotated);
    let _ = std::fs::rename(LOG_FILE, rotated);
}

/// Sanitizes `value` to an identifier-safe token for probe log lines:
/// truncates to 96 chars, replaces everything but ASCII alphanumerics and
/// `-_./:@,` with `_`.
pub fn token(value: &str) -> String {
    compact(value, 96)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '@' | ',') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Sanitizes `value` for embedding in a quoted probe log field: truncates
/// to `max_chars`, replaces everything but printable ASCII (and space)
/// with `_`, so the value can never break out of its surrounding quotes.
pub fn quoted(value: &str, max_chars: usize) -> String {
    compact(value, max_chars)
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' {
                c
            } else if c == ' ' {
                ' '
            } else {
                '_'
            }
        })
        .collect()
}

pub fn compact(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[macro_export]
macro_rules! probe {
    ($tag:expr, $($arg:tt)+) => {{
        #[cfg(debug_assertions)]
        {
            $crate::probe::probe($tag, &::std::format!($($arg)+));
        }
        #[cfg(not(debug_assertions))]
        {
            if false {
                $crate::probe::probe($tag, &::std::format!($($arg)+));
            }
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::{compact, quoted, token};

    #[test]
    fn token_keeps_safe_chars_and_replaces_the_rest() {
        let cases = [
            ("plugin-a_1.2/3:4@5,6", "plugin-a_1.2/3:4@5,6"),
            ("has spaces", "has_spaces"),
            ("quote\"back\\slash", "quote_back_slash"),
        ];
        for (input, expected) in cases {
            assert_eq!(token(input), expected, "input: {input:?}");
        }
    }

    #[test]
    fn token_truncates_to_96_chars() {
        let input = "a".repeat(200);
        assert_eq!(token(&input).chars().count(), 96);
    }

    #[test]
    fn quoted_keeps_printable_ascii_and_space_and_escapes_quote_and_backslash() {
        let cases = [
            ("plain text 123", "plain text 123"),
            ("say \"hi\"", "say _hi_"),
            ("back\\slash", "back_slash"),
            ("tab\tnewline\n", "tab_newline_"),
        ];
        for (input, expected) in cases {
            assert_eq!(quoted(input, 120), expected, "input: {input:?}");
        }
    }

    #[test]
    fn quoted_truncates_to_max_chars() {
        let input = "x".repeat(50);
        assert_eq!(quoted(&input, 10).chars().count(), 10);
    }

    #[test]
    fn compact_takes_at_most_max_chars_and_is_a_no_op_when_shorter() {
        assert_eq!(compact("hello world", 5), "hello");
        assert_eq!(compact("hi", 5), "hi");
    }
}
