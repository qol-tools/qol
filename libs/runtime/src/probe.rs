#[cfg(debug_assertions)]
const LOG_FILE: &str = qol_conventions::TRACE_LOG_PATH;

#[cfg(debug_assertions)]
const MAX_LOG_BYTES: u64 = 16 * 1024 * 1024;

pub fn probe(tag: &str, msg: &str) {
    #[cfg(debug_assertions)]
    {
        emit(sink(), tag, msg);
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (tag, msg);
    }
}

#[cfg(debug_assertions)]
fn emit(sink: &crate::event_tap_trace::TraceSink, tag: &str, msg: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    sink.offer(format!("{ts} pid={} {tag} {msg}\n", std::process::id()));
}

#[cfg(debug_assertions)]
fn sink() -> &'static crate::event_tap_trace::TraceSink {
    static SINK: std::sync::OnceLock<crate::event_tap_trace::TraceSink> =
        std::sync::OnceLock::new();
    SINK.get_or_init(|| {
        let mut file: Option<std::fs::File> = None;
        let mut bytes: u64 = 0;
        crate::event_tap_trace::TraceSink::spawn(
            "qol-probe-log",
            crate::event_tap_trace::QUEUE_DEPTH,
            move |batch| {
                write_batch(
                    &mut file,
                    &mut bytes,
                    LOG_FILE,
                    MAX_LOG_BYTES,
                    &batch.join(""),
                )
            },
        )
    })
}

#[cfg(debug_assertions)]
fn write_batch(
    file: &mut Option<std::fs::File>,
    bytes: &mut u64,
    log_file: &str,
    max_bytes: u64,
    batch: &str,
) {
    use std::io::Write;

    if *bytes >= max_bytes {
        let rotated = format!("{log_file}.1");
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(log_file, rotated);
        *file = None;
        *bytes = 0;
    }
    let f = match file {
        Some(f) => f,
        None => {
            let Ok(opened) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_file)
            else {
                return;
            };
            if let Ok(metadata) = opened.metadata() {
                *bytes = metadata.len();
            }
            file.insert(opened)
        }
    };
    if f.write_all(batch.as_bytes()).is_ok() {
        *bytes += batch.len() as u64;
    }
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
    use super::{compact, emit, quoted, token, write_batch};

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

    #[test]
    fn a_wedged_writer_never_blocks_the_caller() {
        use std::sync::mpsc::channel;
        use std::time::{Duration, Instant};

        let (release, blocked) = channel::<()>();
        let sink = crate::event_tap_trace::TraceSink::spawn(
            "test-probe-wedge",
            crate::event_tap_trace::QUEUE_DEPTH,
            move |_batch| {
                let _ = blocked.recv();
            },
        );

        let started = Instant::now();
        for index in 0..crate::event_tap_trace::QUEUE_DEPTH * 4 {
            emit(&sink, "TEST_WEDGE", &format!("line {index}"));
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(1),
            "a wedged probe writer must never block the caller: {elapsed:?}",
        );
        drop(release);
    }

    #[test]
    fn rotation_renames_the_log_at_the_byte_threshold() {
        use std::io::Read;

        let log = std::env::temp_dir()
            .join(format!("qol-probe-rotation-{}", std::process::id()))
            .to_string_lossy()
            .into_owned();
        let rotated = format!("{log}.1");
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_file(&rotated);

        let mut file: Option<std::fs::File> = None;
        let mut bytes: u64 = 0;
        std::fs::write(&log, "p".repeat(800)).unwrap();
        write_batch(&mut file, &mut bytes, &log, 1000, &"x".repeat(600));
        write_batch(&mut file, &mut bytes, &log, 1000, &"y".repeat(600));
        write_batch(&mut file, &mut bytes, &log, 1000, &"z".repeat(400));

        let mut old = String::new();
        std::fs::File::open(&rotated)
            .expect("the threshold-crossing log must rotate to .1")
            .read_to_string(&mut old)
            .unwrap();
        assert_eq!(old, format!("{}{}", "p".repeat(800), "x".repeat(600)));
        let mut fresh = String::new();
        std::fs::File::open(&log)
            .expect("writes after rotation must reopen a fresh log")
            .read_to_string(&mut fresh)
            .unwrap();
        assert_eq!(fresh, format!("{}{}", "y".repeat(600), "z".repeat(400)));

        drop(file);
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_file(&rotated);
    }
}
