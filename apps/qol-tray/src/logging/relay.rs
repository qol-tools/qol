use std::io::{BufRead, BufReader, Read};
#[cfg(feature = "dev")]
use std::sync::Arc;

#[cfg(feature = "dev")]
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

#[cfg(feature = "dev")]
fn active_patterns(patterns: Vec<String>) -> Option<Arc<Vec<String>>> {
    let active: Vec<String> = patterns.into_iter().filter(|p| !p.is_empty()).collect();
    if active.is_empty() {
        return None;
    }
    Some(Arc::new(active))
}

#[cfg(feature = "dev")]
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

#[cfg(feature = "dev")]
fn relay_lines(reader: impl Read, prefix: &str, suppress: Option<&[String]>, to_stderr: bool) {
    let write: fn(&str) = match to_stderr {
        true => |l| eprint!("{}", l),
        false => |l| print!("{}", l),
    };
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
            write(&line);
        }
    }
}

#[cfg(feature = "dev")]
fn should_suppress(line: &str, patterns: Option<&[String]>) -> bool {
    let Some(patterns) = patterns else {
        return false;
    };
    super::control::matches_any_pattern(line.trim_end(), patterns)
}

#[cfg(not(feature = "dev"))]
pub(crate) fn attach_with_prod_log(
    plugin_id: &str,
    plugin_version: &str,
    plugin_commit: Option<&str>,
    stderr: Option<impl Read + Send + 'static>,
) {
    let Some(stderr) = stderr else { return };
    let source = build_source(plugin_id, plugin_version, plugin_commit);
    let key = format!("plugin.{}.daemon_stderr", plugin_id);
    let id = plugin_id.to_string();
    std::thread::spawn(move || {
        let mut buf = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            let n = buf.read_line(&mut line).unwrap_or(0);
            if n == 0 {
                break;
            }
            let trimmed = line.trim_end();
            if is_error_line(trimmed) {
                crate::log_error!(&key, source = source, "[{}] {}", id, trimmed);
            }
            eprint!("{}", line);
        }
    });
}

#[cfg(not(feature = "dev"))]
fn build_source(plugin_id: &str, version: &str, commit: Option<&str>) -> String {
    match commit {
        Some(c) => format!("plugin:{}@{}@{}", plugin_id, version, c),
        None => format!("plugin:{}@{}", plugin_id, version),
    }
}

#[cfg(not(feature = "dev"))]
fn is_error_line(line: &str) -> bool {
    line.contains("ERROR")
        || line.contains("error")
        || line.contains("FATAL")
        || line.contains("panic")
        || line.contains("PANIC")
}
