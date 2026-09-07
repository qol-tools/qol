use std::fs::{self, File, OpenOptions};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

pub fn is_safe_run_id_component(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id != "."
        && run_id != ".."
        && !run_id.contains('/')
        && !run_id.contains('\\')
}

pub fn lock_run_directory(run_dir: &Path, lock_file_name: &str) -> Result<File> {
    fs::create_dir_all(run_dir)
        .with_context(|| format!("failed to create {}", run_dir.display()))?;
    let path = run_dir.join(lock_file_name);
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    lock.lock()
        .with_context(|| format!("failed to lock {}", path.display()))?;
    Ok(lock)
}

pub fn write_json_report(path: &Path, value: &Value) -> Result<()> {
    let content = serde_json::to_string_pretty(value).context("failed to serialize JSON")?;
    qol_fs::atomic_write(path, format!("{content}\n").as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))
}

pub fn remove_unpublished_run_dir(run_dir: &Path, kind: &str) -> Result<()> {
    match fs::remove_dir_all(run_dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove unpublished {kind} {}", run_dir.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_run_id_rejects_traversal_and_empty() {
        let cases = [
            ("run-123", true),
            ("run_abc.2", true),
            ("", false),
            (".", false),
            ("..", false),
            ("a/b", false),
            ("a\\b", false),
        ];
        for (run_id, expected) in cases {
            assert_eq!(
                is_safe_run_id_component(run_id),
                expected,
                "run_id: {run_id:?}"
            );
        }
    }

    #[test]
    fn lock_run_directory_creates_dir_and_locks_file() {
        let dir = tempfile::tempdir().unwrap();
        let run_dir = dir.path().join("run-a");

        let lock = lock_run_directory(&run_dir, "reconcile.lock").unwrap();

        assert!(run_dir.join("reconcile.lock").exists());
        drop(lock);
    }

    #[test]
    fn write_json_report_writes_pretty_json_with_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");

        write_json_report(&path, &serde_json::json!({"status": "ok"})).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.ends_with('\n'));
        assert_eq!(
            serde_json::from_str::<Value>(&content).unwrap(),
            serde_json::json!({"status": "ok"})
        );
    }

    #[test]
    fn remove_unpublished_run_dir_tolerates_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let run_dir = dir.path().join("does-not-exist");

        remove_unpublished_run_dir(&run_dir, "flow").unwrap();
    }

    #[test]
    fn remove_unpublished_run_dir_removes_existing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let run_dir = dir.path().join("run-a");
        fs::create_dir_all(&run_dir).unwrap();
        fs::write(run_dir.join("report.json"), b"{}").unwrap();

        remove_unpublished_run_dir(&run_dir, "environment batch").unwrap();

        assert!(!run_dir.exists());
    }
}
