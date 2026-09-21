use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{emu_run_line, strip_ansi, LOG_CAP};

pub(super) struct LogRing {
    pub(super) lines: VecDeque<String>,
    collapse: bool,
    last_key: Option<String>,
    repeat: usize,
}

impl LogRing {
    pub(super) fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            collapse: false,
            last_key: None,
            repeat: 0,
        }
    }

    pub(super) fn collapsing() -> Self {
        Self {
            collapse: true,
            ..Self::new()
        }
    }

    pub(super) fn push(&mut self, line: String) {
        if self.collapse && self.try_collapse(&line) {
            return;
        }
        if self.lines.len() == LOG_CAP {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    fn try_collapse(&mut self, line: &str) -> bool {
        let key = collapse_key(line);
        if self.repeat > 0 && self.last_key.as_deref() == Some(key.as_str()) {
            if let Some(slot) = self.lines.back_mut() {
                self.repeat += 1;
                *slot = format!("{line}{}", repeat_badge(self.repeat));
                return true;
            }
        }
        self.last_key = Some(key);
        self.repeat = 1;
        false
    }

    pub(super) fn len(&self) -> usize {
        self.lines.len()
    }
}

fn repeat_badge(count: usize) -> String {
    format!("\u{1b}[2m (\u{d7}{count})\u{1b}[0m")
}

pub(super) fn collapse_key(line: &str) -> String {
    let plain = strip_ansi(line);
    let trimmed = plain.trim();
    trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .map(|(_, tail)| tail.trim().to_string())
        .unwrap_or_else(|| trimmed.to_string())
}

fn shape_key(line: &str) -> String {
    let plain = strip_ansi(line);
    let trimmed = plain.trim();
    let after_ts = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .map(|(_, tail)| tail)
        .unwrap_or(trimmed);
    if let Some(open) = after_ts.find('[') {
        if let Some((source, tail)) = after_ts[open + 1..].split_once(']') {
            let tag = tail
                .split([' ', '\t', ':'])
                .find(|token| !token.is_empty())
                .unwrap_or("");
            return format!("[{source}] {tag}");
        }
    }
    after_ts
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn window_start(len: usize, height: usize, offset: usize) -> usize {
    len.saturating_sub(height.saturating_add(offset))
}

pub(super) fn clamp_offset(len: usize, height: usize, offset: usize) -> usize {
    offset.min(len.saturating_sub(height))
}

struct OwnedSource {
    child: Child,
    rx: Receiver<String>,
}

pub(super) struct LogPane {
    pub(super) ring: LogRing,
    source: Option<OwnedSource>,
    shape_last_emit: HashMap<String, Instant>,
}

impl LogPane {
    pub(super) fn new() -> Self {
        Self {
            ring: LogRing::new(),
            source: None,
            shape_last_emit: HashMap::new(),
        }
    }

    pub(super) fn collapsing() -> Self {
        Self {
            ring: LogRing::collapsing(),
            source: None,
            shape_last_emit: HashMap::new(),
        }
    }

    pub(super) fn attach(&mut self, child: Child, rx: Receiver<String>) {
        self.source = Some(OwnedSource { child, rx });
    }

    pub(super) fn is_live(&self) -> bool {
        self.source.is_some()
    }

    pub(super) fn push(&mut self, line: String) {
        self.ring.push(line);
    }

    pub(super) fn len(&self) -> usize {
        self.ring.len()
    }

    pub(super) fn drain_rated(
        &mut self,
        keep: impl Fn(&str) -> bool,
        realtime: bool,
        now: Instant,
        min_interval: Duration,
    ) {
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let mut received = Vec::new();
        while let Ok(line) = source.rx.try_recv() {
            received.push(line);
        }
        for line in received {
            if !keep(&line) {
                continue;
            }
            if !realtime {
                let shape = shape_key(&line);
                let throttled = self
                    .shape_last_emit
                    .get(&shape)
                    .is_some_and(|last| now.duration_since(*last) < min_interval);
                if throttled {
                    continue;
                }
                self.shape_last_emit.insert(shape, now);
            }
            self.ring.push(line);
        }
    }

    pub(super) fn stop(&mut self) {
        if let Some(mut source) = self.source.take() {
            let _ = source.child.kill();
            let _ = source.child.wait();
        }
    }

    pub(super) fn replay(path: &Path) -> Self {
        let mut ring = LogRing::new();
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                ring.push(line.to_string());
            }
        }
        Self {
            ring,
            source: None,
            shape_last_emit: HashMap::new(),
        }
    }

    pub(super) fn poll_finished(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let mut received = Vec::new();
        let mut exited = None;
        if let Some(source) = self.source.as_mut() {
            while let Ok(line) = source.rx.try_recv() {
                received.push(line);
            }
            if let Ok(Some(status)) = source.child.try_wait() {
                while let Ok(line) = source.rx.try_recv() {
                    received.push(line);
                }
                exited = Some(status);
            }
        }
        for line in received {
            if keep(&line) {
                self.ring.push(line);
            }
        }
        match exited {
            Some(status) => {
                self.ring.push(emu_run_line("done", &status.to_string()));
                self.source = None;
                true
            }
            None => false,
        }
    }

    pub(super) fn wait_for_exit_until(&mut self, deadline: Instant) -> bool {
        let Some(source) = self.source.as_mut() else {
            return true;
        };
        loop {
            match source.child.try_wait() {
                Ok(Some(_)) => {
                    self.source = None;
                    return true;
                }
                Ok(None) => {}
                Err(_) => return false,
            }
            if Instant::now() >= deadline {
                return false;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            std::thread::sleep(remaining.min(Duration::from_millis(100)));
        }
    }
}

pub(super) struct DevLogFile {
    pub(super) path: PathBuf,
    writer: Option<BufWriter<File>>,
}

impl DevLogFile {
    pub(super) fn create() -> Option<Self> {
        let primary = dev_log_dir();
        create_dev_log_file_in(&primary).or_else(|| {
            let fallback = std::env::temp_dir().join("qol-tray/logs");
            create_dev_log_file_in(&fallback)
        })
    }

    #[cfg(test)]
    pub(super) fn path_only(path: PathBuf) -> Self {
        Self { path, writer: None }
    }

    pub(super) fn write_line(&mut self, line: &str) {
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        let _ = writeln!(writer, "{line}");
        let _ = writer.flush();
    }
}

pub(super) fn dev_log_dir() -> PathBuf {
    qol_log::log_dir()
}

fn create_dev_log_file_in(dir: &Path) -> Option<DevLogFile> {
    fs::create_dir_all(dir).ok()?;
    // Every qol dev run opens its own file, so nothing here ever rotates;
    // sweep the older runs before adding one more.
    let _ = qol_log::prune_matching(dir, "qol-dev", qol_log::FILES_KEPT);
    let path = dir.join(dev_log_file_name());
    let writer = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
        .map(BufWriter::new)?;
    Some(DevLogFile {
        path,
        writer: Some(writer),
    })
}

fn dev_log_file_name() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("qol-dev-{ts}-{}.log", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::dev_console::*;

    #[test]
    fn coordinator_wait_child_fixture() {
        if std::env::var_os("QOL_LOG_PANE_WAIT_FIXTURE").is_some() {
            std::thread::sleep(Duration::from_secs(30));
        }
    }

    #[test]
    fn bounded_wait_does_not_kill_coordinator_without_guest_proof() {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "dev_console::log_pane::tests::coordinator_wait_child_fixture",
                "--nocapture",
            ])
            .env("QOL_LOG_PANE_WAIT_FIXTURE", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut pane = LogPane::new();
        pane.attach(child, rx);

        assert!(!pane.wait_for_exit_until(Instant::now() + Duration::from_millis(50)));
        assert!(pane.is_live());
        assert!(matches!(
            pane.source.as_mut().unwrap().child.try_wait(),
            Ok(None)
        ));

        pane.stop();
    }

    #[test]
    fn collapse_key_ignores_timestamp_and_ansi() {
        let a = collapse_key("\u{1b}[90m[10:00:00.001]\u{1b}[0m GHOSTWIN foo");
        let b = collapse_key("\u{1b}[90m[10:00:00.999]\u{1b}[0m GHOSTWIN foo");
        assert_eq!(a, b, "the same event at different times shares a key");
        assert_eq!(a, "GHOSTWIN foo");
        assert_ne!(
            a,
            collapse_key("[10:00:00.001] FOCUS bar"),
            "distinct events have distinct keys"
        );
    }

    #[test]
    fn collapsing_ring_folds_identical_consecutive_lines_with_a_count() {
        let mut ring = LogRing::collapsing();
        for ms in ["001", "050", "120"] {
            ring.push(format!("\u{1b}[90m[10:00:00.{ms}]\u{1b}[0m GHOSTWIN foo"));
        }
        assert_eq!(
            ring.len(),
            1,
            "events differing only by timestamp collapse to one line"
        );
        let folded = strip_ansi(&ring.lines[0]);
        assert!(folded.contains("GHOSTWIN foo"), "keeps the text: {folded}");
        assert!(
            folded.contains("(\u{d7}3)"),
            "shows the repeat count: {folded}"
        );

        ring.push("[10:00:00.200] FOCUS bar".to_string());
        assert_eq!(ring.len(), 2, "a distinct event starts a new line");

        ring.push("[10:00:00.260] FOCUS bar".to_string());
        assert_eq!(ring.len(), 2, "the new event collapses on repeat too");
        assert!(strip_ansi(&ring.lines[1]).contains("(\u{d7}2)"));
    }

    #[test]
    fn non_collapsing_ring_keeps_every_line() {
        let mut ring = LogRing::new();
        ring.push("[10:00:00.001] GHOSTWIN foo".to_string());
        ring.push("[10:00:00.050] GHOSTWIN foo".to_string());
        assert_eq!(ring.len(), 2, "the logs ring never folds repeats");
    }

    #[test]
    fn log_ring_caps_and_evicts_oldest() {
        let mut ring = LogRing::new();
        for i in 0..(LOG_CAP + 3) {
            ring.push(format!("line-{i}"));
        }
        assert_eq!(ring.len(), LOG_CAP);
        assert_eq!(ring.lines.front().map(String::as_str), Some("line-3"));
    }

    #[test]
    fn window_math_keeps_tail_and_clamps() {
        let cases = [
            ("follow shows tail", 100, 10, 0, 90),
            ("scrolled up shifts window", 100, 10, 25, 65),
            ("short log starts at zero", 5, 10, 0, 0),
            ("offset beyond start clamps to zero", 100, 10, 500, 0),
        ];
        for (label, len, height, offset, want_start) in cases {
            assert_eq!(window_start(len, height, offset), want_start, "{label}");
        }
        assert_eq!(clamp_offset(100, 10, 500), 90, "clamp to len-height");
        assert_eq!(clamp_offset(5, 10, 3), 0, "short log clamps to zero");
    }

    #[test]
    fn creating_a_dev_log_file_prunes_older_runs() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..(qol_log::FILES_KEPT + 3) {
            std::fs::write(dir.path().join(format!("qol-dev-{n}-{n}.log")), "old run").unwrap();
        }
        std::fs::write(dir.path().join("unrelated.log"), "keep me").unwrap();

        let created = create_dev_log_file_in(dir.path()).unwrap();

        assert!(created.path.starts_with(dir.path()));
        assert!(created.path.exists());
        assert!(dir.path().join("unrelated.log").exists());
        let run_files = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with("qol-dev-") && name.ends_with(".log")
            })
            .count();
        assert_eq!(run_files, qol_log::FILES_KEPT + 1);
    }
}
