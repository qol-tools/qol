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

fn spawn_log_relay<R>(
    plugin_id: String,
    stream_name: &'static str,
    reader: R,
    suppress_patterns: Option<Arc<Vec<String>>>,
    to_stderr: bool,
) where
    R: std::io::Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            let read = match reader.read_line(&mut line) {
                Ok(read) => read,
                Err(error) => {
                    log::debug!(
                        "Plugin daemon log relay failed for {} ({}): {}",
                        plugin_id,
                        stream_name,
                        error
                    );
                    break;
                }
            };
            if read == 0 {
                break;
            }
            if should_suppress_line(&line, suppress_patterns.as_ref()) {
                continue;
            }
            print_relay_line(&line, to_stderr);
        }
    });
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
