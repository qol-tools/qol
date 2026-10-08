use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

const RECORD_FILE: &str = ".restart-record.json";
const LEGACY_UPDATE_MARKER: &str = ".update-from-version";
const RESUMES_BEFORE_GIVING_UP: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RestartRecord {
    pub(crate) from_version: String,
    #[serde(default)]
    pub(crate) attempts: u32,
    #[serde(default)]
    pub(crate) update_plugins: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Resume {
    pub(crate) updated_from: Option<String>,
    pub(crate) update_plugins: Vec<String>,
    pub(crate) dropped_plugins: Vec<String>,
}

pub(crate) fn record_path(dir: &Path) -> PathBuf {
    dir.join(RECORD_FILE)
}

pub(crate) fn write(dir: &Path, record: &RestartRecord) -> io::Result<PathBuf> {
    let path = record_path(dir);
    let json = serde_json::to_vec(record).map_err(io::Error::other)?;
    qol_fs::atomic_write_durable(&path, &json)?;
    Ok(path)
}

pub(crate) fn begin(dir: &Path) -> Option<RestartRecord> {
    let mut record = read(dir)?;
    record.attempts = record.attempts.saturating_add(1);
    if let Err(error) = write(dir, &record) {
        log::warn!("Failed to count the restart attempt, not resuming its work: {error}");
        record.update_plugins.clear();
    }
    Some(record)
}

pub(crate) fn finish(dir: &Path) {
    match std::fs::remove_file(record_path(dir)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("Failed to delete the restart record: {error}"),
    }
}

pub(crate) fn resume_plan(record: &RestartRecord, current_version: &str) -> Resume {
    let from = record.from_version.trim();
    if from.is_empty() || from == current_version {
        return Resume::default();
    }
    let gave_up = record.attempts > RESUMES_BEFORE_GIVING_UP;
    let (update_plugins, dropped_plugins) = if gave_up {
        (Vec::new(), record.update_plugins.clone())
    } else {
        (record.update_plugins.clone(), Vec::new())
    };
    Resume {
        updated_from: Some(from.to_string()),
        update_plugins,
        dropped_plugins,
    }
}

fn read(dir: &Path) -> Option<RestartRecord> {
    let path = record_path(dir);
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(record) => Some(record),
            Err(error) => {
                log::warn!("Discarding an unreadable restart record: {error}");
                finish(dir);
                None
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => read_legacy_marker(dir),
        Err(error) => {
            log::warn!("Failed to read the restart record: {error}");
            None
        }
    }
}

fn read_legacy_marker(dir: &Path) -> Option<RestartRecord> {
    let marker = dir.join(LEGACY_UPDATE_MARKER);
    let from = std::fs::read_to_string(&marker).ok()?;
    let _ = std::fs::remove_file(&marker);
    Some(RestartRecord {
        from_version: from.trim().to_string(),
        ..RestartRecord::default()
    })
}

#[cfg(test)]
mod tests;
