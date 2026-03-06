use crate::plugins::Plugin;
use std::io::{BufRead, BufReader};
use std::process::Child;
use std::sync::Arc;

pub(super) fn attach_filtered_log_relay(
    plugin: &Plugin,
    child: &mut Child,
    suppress_patterns: Vec<String>,
) {
    let patterns = active_patterns(suppress_patterns);
    if let Some(stdout) = child.stdout.take() {
        spawn_log_relay(plugin.id.clone(), "stdout", stdout, patterns.clone(), false);
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_log_relay(plugin.id.clone(), "stderr", stderr, patterns, true);
    }
}

fn active_patterns(patterns: Vec<String>) -> Option<Arc<Vec<String>>> {
    let active_patterns: Vec<String> = patterns
        .into_iter()
        .filter(|pattern| !pattern.is_empty())
        .collect();
    if active_patterns.is_empty() {
        return None;
    }
    Some(Arc::new(active_patterns))
}

fn spawn_log_relay<R: std::io::Read + Send + 'static>(
    plugin_id: String,
    stream_name: &'static str,
    reader: R,
    suppress_patterns: Option<Arc<Vec<String>>>,
    to_stderr: bool,
) {
    std::thread::spawn(move || {
        let log_prefix = format!("{} ({})", plugin_id, stream_name);
        relay_lines(reader, &log_prefix, suppress_patterns.as_ref(), to_stderr);
    });
}

fn relay_lines(
    reader: impl std::io::Read,
    log_prefix: &str,
    suppress: Option<&Arc<Vec<String>>>,
    to_stderr: bool,
) {
    let mut buf = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        let n = buf.read_line(&mut line).unwrap_or_else(|e| {
            log::debug!("Plugin daemon log relay failed for {}: {}", log_prefix, e);
            0
        });
        if n == 0 { break }
        if !should_suppress_line(&line, suppress) {
            print_relay_line(&line, to_stderr);
        }
    }
}

fn should_suppress_line(line: &str, patterns: Option<&Arc<Vec<String>>>) -> bool {
    let Some(patterns) = patterns else {
        return false;
    };
    let trimmed = line.trim_end();
    patterns
        .iter()
        .any(|pattern| trimmed.contains(pattern.as_str()))
}

fn print_relay_line(line: &str, to_stderr: bool) {
    if to_stderr {
        eprint!("{}", line);
        return;
    }
    print!("{}", line);
}
