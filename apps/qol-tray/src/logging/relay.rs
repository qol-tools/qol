#[cfg(feature = "dev")]
use std::io::Write;
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
        spawn_relay(label.to_owned(), stdout, patterns.clone(), false);
    }
    if let Some(stderr) = stderr {
        spawn_relay(label.to_owned(), stderr, patterns, true);
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
    reader: R,
    suppress_patterns: Option<Arc<Vec<String>>>,
    to_stderr: bool,
) {
    std::thread::spawn(move || {
        let mut sink = daemon_log_sink();
        let file = sink.as_mut().map(|file| file as &mut dyn Write);
        let suppress = suppress_patterns.as_deref().map(Vec::as_slice);
        if to_stderr {
            relay_lines(reader, &label, suppress, std::io::stderr(), file);
        } else {
            relay_lines(reader, &label, suppress, std::io::stdout(), file);
        }
    });
}

#[cfg(feature = "dev")]
fn daemon_log_sink() -> Option<std::fs::File> {
    let dir = super::platform::log_dir();
    let _ = std::fs::create_dir_all(&dir);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("qol-daemons.log"))
        .ok()
}

// The console sink is the tray's own stdio, which is a pipe into qol dev and
// dies at every generation handoff. Writes there must be fire-and-forget:
// println!-style macros panic on EPIPE, and that panic was aborting relay
// threads (and, via inherited stdio, entire daemons). The file sink is the
// durable copy that survives the handoff.
#[cfg(feature = "dev")]
fn relay_lines(
    reader: impl Read,
    label: &str,
    suppress: Option<&[String]>,
    mut console: impl Write,
    mut file: Option<&mut dyn Write>,
) {
    let mut buf = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        let n = buf.read_line(&mut line).unwrap_or_else(|error| {
            log::debug!("Log relay failed for {}: {}", label, error);
            0
        });
        if n == 0 {
            break;
        }
        if should_suppress(&line, suppress) {
            continue;
        }
        let _ = console.write_all(line.as_bytes());
        let _ = console.flush();
        if let Some(file) = file.as_deref_mut() {
            let _ = file.write_all(format!("[{label}] {line}").as_bytes());
            let _ = file.flush();
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

#[cfg(all(test, feature = "dev"))]
mod tests {
    use super::*;

    #[test]
    fn relay_tees_lines_to_console_and_file_with_label() {
        let mut console = Vec::new();
        let mut file = Vec::new();

        relay_lines(
            "alpha\nbeta\n".as_bytes(),
            "plugin-foo",
            None,
            &mut console,
            Some(&mut file),
        );

        assert_eq!(String::from_utf8(console).unwrap(), "alpha\nbeta\n");
        assert_eq!(
            String::from_utf8(file).unwrap(),
            "[plugin-foo] alpha\n[plugin-foo] beta\n"
        );
    }

    #[test]
    fn relay_suppressed_lines_reach_neither_sink() {
        let patterns = vec!["beta".to_string()];
        let mut console = Vec::new();
        let mut file = Vec::new();

        relay_lines(
            "alpha\nbeta\n".as_bytes(),
            "plugin-foo",
            Some(&patterns),
            &mut console,
            Some(&mut file),
        );

        assert_eq!(String::from_utf8(console).unwrap(), "alpha\n");
        assert_eq!(String::from_utf8(file).unwrap(), "[plugin-foo] alpha\n");
    }

    struct DeadPipe;

    impl Write for DeadPipe {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
    }

    // Regression test for the generation-handoff abort class: qol dev
    // exec-replaces itself and the console pipe dies; the relay must keep
    // draining into the file without panicking, or daemon output backs up
    // and the writes that used to go through println!-style macros abort.
    #[test]
    fn relay_survives_a_dead_console_sink() {
        let mut file = Vec::new();

        relay_lines(
            "alpha\nbeta\n".as_bytes(),
            "plugin-foo",
            None,
            DeadPipe,
            Some(&mut file),
        );

        assert_eq!(
            String::from_utf8(file).unwrap(),
            "[plugin-foo] alpha\n[plugin-foo] beta\n",
            "a dead console pipe must not stop or panic the relay; the file sink \
             is the durable record"
        );
    }
}
