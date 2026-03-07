use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;

pub(crate) fn attach(
    label: &str,
    stdout: Option<impl Read + Send + 'static>,
    stderr: Option<impl Read + Send + 'static>,
    suppress_patterns: Vec<String>,
) {
    let patterns = active_patterns(suppress_patterns);
    if let Some(stdout) = stdout {
        spawn_relay(label.to_owned(), "stdout", stdout, patterns.clone(), false);
    }
    if let Some(stderr) = stderr {
        spawn_relay(label.to_owned(), "stderr", stderr, patterns, true);
    }
}

fn active_patterns(patterns: Vec<String>) -> Option<Arc<Vec<String>>> {
    let active: Vec<String> = patterns.into_iter().filter(|p| !p.is_empty()).collect();
    if active.is_empty() {
        return None;
    }
    Some(Arc::new(active))
}

fn spawn_relay<R: Read + Send + 'static>(
    label: String,
    stream_name: &'static str,
    reader: R,
    suppress_patterns: Option<Arc<Vec<String>>>,
    to_stderr: bool,
) {
    std::thread::spawn(move || {
        let prefix = format!("{} ({})", label, stream_name);
        relay_lines(
            reader,
            &prefix,
            suppress_patterns.as_deref().map(Vec::as_slice),
            to_stderr,
        );
    });
}

fn relay_lines(reader: impl Read, prefix: &str, suppress: Option<&[String]>, to_stderr: bool) {
    let mut buf = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        let n = buf.read_line(&mut line).unwrap_or_else(|e| {
            log::debug!("Log relay failed for {}: {}", prefix, e);
            0
        });
        if n == 0 {
            break;
        }
        if !should_suppress(&line, suppress) {
            if to_stderr {
                eprint!("{}", line);
            } else {
                print!("{}", line);
            }
        }
    }
}

fn should_suppress(line: &str, patterns: Option<&[String]>) -> bool {
    let Some(patterns) = patterns else {
        return false;
    };
    super::control::matches_any_pattern(line.trim_end(), patterns)
}
