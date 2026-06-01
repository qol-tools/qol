use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const SENTINEL_FILE: &str = "needs-doctor.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Trigger {
    pub reason: String,
    pub check_id: String,
    pub written_at: SystemTime,
}

pub fn mark_needed(check_id: &str, reason: &str) -> Result<()> {
    let path = sentinel_path()?;
    write_sentinel_at(&path, check_id, reason)
}

pub fn take() -> Option<Trigger> {
    let path = sentinel_path().ok()?;
    take_at(&path)
}

fn sentinel_path() -> Result<PathBuf> {
    crate::paths::base_data_dir().map(|dir| dir.join(SENTINEL_FILE))
}

fn write_sentinel_at(path: &Path, check_id: &str, reason: &str) -> Result<()> {
    let trigger = Trigger {
        reason: reason.to_string(),
        check_id: check_id.to_string(),
        written_at: SystemTime::now(),
    };
    let parent = path
        .parent()
        .with_context(|| format!("sentinel path {} has no parent", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let body = serde_json::to_vec_pretty(&trigger).context("failed to serialize trigger")?;
    let tmp = tmp_path(path);
    fs::write(&tmp, &body).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("failed to rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

fn take_at(path: &Path) -> Option<Trigger> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn!("doctor trigger: read {} failed: {error}", path.display());
            return None;
        }
    };
    let parsed: Result<Trigger, _> = serde_json::from_slice(&bytes);
    let _ = fs::remove_file(path);
    match parsed {
        Ok(trigger) => Some(trigger),
        Err(error) => {
            log::warn!(
                "doctor trigger: corrupt sentinel at {}: {error}",
                path.display()
            );
            None
        }
    }
}

fn tmp_path(target: &Path) -> PathBuf {
    let file = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "trigger".to_string());
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    target.with_file_name(format!(".{file}.{pid}.{nanos}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_path() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(SENTINEL_FILE);
        (dir, path)
    }

    #[test]
    fn round_trip_returns_written_trigger_then_clears() {
        let (_dir, path) = fresh_path();
        write_sentinel_at(&path, "hotkey_shadows", "Super+Space failed").expect("write");
        let trigger = take_at(&path).expect("trigger present");
        assert_eq!(trigger.check_id, "hotkey_shadows");
        assert_eq!(trigger.reason, "Super+Space failed");
        assert!(take_at(&path).is_none(), "second take must be None");
        assert!(!path.exists(), "sentinel file must be removed by take");
    }

    #[test]
    fn take_on_missing_file_is_none() {
        let (_dir, path) = fresh_path();
        assert!(take_at(&path).is_none());
    }

    #[test]
    fn take_on_corrupt_file_is_none_and_removes_file() {
        let (_dir, path) = fresh_path();
        fs::write(&path, b"{not valid json").expect("write garbage");
        assert!(take_at(&path).is_none());
        assert!(
            !path.exists(),
            "corrupt sentinel must still be removed so it doesn't loop"
        );
    }

    #[test]
    fn write_overwrites_previous_sentinel_atomically() {
        let (_dir, path) = fresh_path();
        write_sentinel_at(&path, "first", "one").expect("first write");
        write_sentinel_at(&path, "second", "two").expect("second write");
        let trigger = take_at(&path).expect("trigger present");
        assert_eq!(trigger.check_id, "second", "later write must win");
        assert_eq!(trigger.reason, "two");
    }

    #[test]
    fn partial_tmp_file_does_not_break_take() {
        let (_dir, path) = fresh_path();
        let stray_tmp = path.with_file_name(format!(".{SENTINEL_FILE}.123.999.tmp"));
        fs::write(&stray_tmp, b"\x00partial").expect("partial");
        assert!(
            take_at(&path).is_none(),
            "take must ignore stray tmp and only look at the real path"
        );
        assert!(
            stray_tmp.exists(),
            "stray tmp is not the doctor's responsibility to clean up"
        );
    }
}
