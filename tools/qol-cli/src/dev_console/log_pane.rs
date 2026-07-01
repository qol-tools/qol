use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{core_log_dir, emu_run_line, strip_ansi, LOG_CAP, STOP_GRACE};

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

    pub(super) fn stop_graceful(&mut self) {
        let Some(mut source) = self.source.take() else {
            return;
        };
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            if matches!(source.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = source.child.kill();
        let _ = source.child.wait();
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
    core_log_dir()
}

fn create_dev_log_file_in(dir: &Path) -> Option<DevLogFile> {
    fs::create_dir_all(dir).ok()?;
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
