use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use qol_windowing::{WindowId, WindowRect};

use super::{MinimizedStateStore, MinimizedWindowRecord};

pub(crate) const LAST_MINIMIZED_WINDOW_FILE_NAME: &str = "qol-window-actions-last-minimized";

pub(crate) struct FileMinimizedStateStore {
    path: PathBuf,
}

impl FileMinimizedStateStore {
    pub(crate) fn new(path: PathBuf) -> Self {
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
        format!("|{},{},{},{}", r.x, r.y, r.width, r.height)
    });
    format!(
        "{}|{}|{}|{}{}",
        record.window_id.as_str(),
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
    let window_id = WindowId::parse(parts.next()?.trim())?;
    let pid = parts.next()?.trim().parse::<u32>().ok()?;
    let process_start_ticks = parts.next()?.trim().parse::<u64>().ok()?;
    let saved_at_unix_secs = parts.next()?.trim().parse::<u64>().ok()?;

    let saved_rect = parts.next().and_then(|s| {
        let mut r = s.split(',');
        let x = r.next()?.parse::<f64>().ok()?;
        let y = r.next()?.parse::<f64>().ok()?;
        let w = r.next()?.parse::<f64>().ok()?;
        let h = r.next()?.parse::<f64>().ok()?;
        Some(WindowRect::from_array([x, y, w, h]))
    });

    Some(MinimizedWindowRecord {
        window_id,
        pid,
        process_start_ticks,
        saved_at_unix_secs,
        saved_rect,
    })
}
