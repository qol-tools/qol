use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use crate::restore::{MinimizedStateStore, MinimizedWindowRecord};

const LAST_MINIMIZED_WINDOW_FILE_NAME: &str = "qol-window-actions-last-minimized";

pub struct FileMinimizedStateStore {
    path: PathBuf,
}

impl FileMinimizedStateStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl MinimizedStateStore for FileMinimizedStateStore {
    fn peek(&self) -> Result<Option<MinimizedWindowRecord>, String> {
        let lines = match read_lines(&self.path) {
            Ok(l) => l,
            Err(None) => return Ok(None),
            Err(Some(e)) => return Err(e),
        };
        Ok(lines.last().and_then(|l| parse_record(l)))
    }

    fn push(&self, record: &MinimizedWindowRecord) {
        let mut lines = read_lines(&self.path).unwrap_or_default();
        lines.push(serialize_record(record));
        write_lines(&self.path, &lines);
    }

    fn pop(&self) -> Result<Option<MinimizedWindowRecord>, String> {
        let mut lines = match read_lines(&self.path) {
            Ok(l) => l,
            Err(None) => return Ok(None),
            Err(Some(e)) => return Err(e),
        };
        let last = lines.pop().and_then(|l| parse_record(&l));
        if lines.is_empty() {
            let _ = fs::remove_file(&self.path);
        } else {
            write_lines(&self.path, &lines);
        }
        Ok(last)
    }
}

pub fn default_state_file_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    let base = env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    #[cfg(not(target_os = "macos"))]
    let base = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(LAST_MINIMIZED_WINDOW_FILE_NAME)
}

fn read_lines(path: &PathBuf) -> Result<Vec<String>, Option<String>> {
    match fs::read_to_string(path) {
        Ok(content) => {
            let lines: Vec<String> = content
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(String::from)
                .collect();
            if lines.is_empty() {
                Err(None)
            } else {
                Ok(lines)
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Err(None),
        Err(e) => Err(Some(format!("Failed to read minimized window state: {e}"))),
    }
}

fn write_lines(path: &PathBuf, lines: &[String]) {
    let content = lines.join("\n") + "\n";
    let _ = fs::write(path, content.as_bytes());
}

fn serialize_record(record: &MinimizedWindowRecord) -> String {
    let rect_str = record.saved_rect.map_or_else(String::new, |r| {
        format!("|{},{},{},{}", r[0], r[1], r[2], r[3])
    });
    format!(
        "{}|{}|{}|{}{}",
        record.window_id,
        record.pid,
        record.process_start_ticks,
        record.saved_at_unix_secs,
        rect_str,
    )
}

fn parse_record(raw: &str) -> Option<MinimizedWindowRecord> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = trimmed.split('|');
    let window_id = normalize_window_id(parts.next()?.trim())?;
    let pid = parts.next()?.trim().parse::<u32>().ok()?;
    let process_start_ticks = parts.next()?.trim().parse::<u64>().ok()?;
    let saved_at_unix_secs = parts.next()?.trim().parse::<u64>().ok()?;

    let saved_rect = parts.next().and_then(|s| {
        let mut r = s.split(',');
        let x = r.next()?.parse::<f64>().ok()?;
        let y = r.next()?.parse::<f64>().ok()?;
        let w = r.next()?.parse::<f64>().ok()?;
        let h = r.next()?.parse::<f64>().ok()?;
        Some([x, y, w, h])
    });

    Some(MinimizedWindowRecord {
        window_id,
        pid,
        process_start_ticks,
        saved_at_unix_secs,
        saved_rect,
    })
}

fn normalize_window_id(window_id: &str) -> Option<String> {
    if window_id.starts_with("pid:") {
        return Some(window_id.to_string());
    }
    if is_x11_window_id(window_id) {
        return Some(window_id.to_ascii_lowercase());
    }
    let numeric = window_id.trim().parse::<u64>().ok()?;
    Some(format!("0x{numeric:x}"))
}

fn is_x11_window_id(id: &str) -> bool {
    id.starts_with("0x") && id.len() > 2 && id.chars().skip(2).all(|c| c.is_ascii_hexdigit())
}
