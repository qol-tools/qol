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
    fn read(&self) -> Result<Option<MinimizedWindowRecord>, String> {
        match fs::read_to_string(&self.path) {
            Ok(value) => {
                let parsed = parse_minimized_window_record(&value);
                if parsed.is_none() && !value.trim().is_empty() {
                    self.clear();
                }
                Ok(parsed)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("Failed to read minimized window state: {error}")),
        }
    }

    fn write(&self, record: &MinimizedWindowRecord) {
        let line = format!(
            "{}|{}|{}|{}\n",
            record.window_id, record.pid, record.process_start_ticks, record.saved_at_unix_secs
        );
        let _ = fs::write(&self.path, line.as_bytes());
    }

    fn clear(&self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn default_state_file_path() -> PathBuf {
    let base = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(LAST_MINIMIZED_WINDOW_FILE_NAME)
}

fn parse_minimized_window_record(raw: &str) -> Option<MinimizedWindowRecord> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = trimmed.split('|');
    let window_id = normalize_window_id(parts.next()?.trim())?;
    let pid = parts.next()?.trim().parse::<u32>().ok()?;
    let process_start_ticks = parts.next()?.trim().parse::<u64>().ok()?;
    let saved_at_unix_secs = parts.next()?.trim().parse::<u64>().ok()?;

    if parts.next().is_some() {
        return None;
    }

    Some(MinimizedWindowRecord {
        window_id,
        pid,
        process_start_ticks,
        saved_at_unix_secs,
    })
}

fn normalize_window_id(window_id: &str) -> Option<String> {
    if is_window_id(window_id) {
        return Some(window_id.to_ascii_lowercase());
    }

    let numeric = window_id.trim().parse::<u64>().ok()?;
    Some(format!("0x{numeric:x}"))
}

fn is_window_id(id: &str) -> bool {
    id.starts_with("0x") && id.chars().skip(2).all(|c| c.is_ascii_hexdigit())
}
