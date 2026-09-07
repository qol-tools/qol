use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

mod platform;

pub use tracing_appender;

/// Number of log files any qol sink keeps. One policy, one home.
pub const FILES_KEPT: usize = 7;

fn dir_override() -> &'static OnceLock<PathBuf> {
    static OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
    &OVERRIDE
}

/// Redirects `log_dir()` for this process. First call wins; later calls are
/// ignored. qol-tray uses this to keep its test-path-root redirection.
pub fn set_dir_override(dir: PathBuf) {
    let _ = dir_override().set(dir);
}

/// The directory every qol process writes logs into. Honours an override
/// set by `set_dir_override`, otherwise the per-OS default.
pub fn log_dir() -> PathBuf {
    dir_override()
        .get()
        .cloned()
        .unwrap_or_else(platform::log_dir)
}

/// A bounded appender for a continuous sink: rotates daily and keeps
/// `FILES_KEPT` files, writing `<prefix>.<date>.log` in `dir`.
pub fn rolling(dir: &Path, prefix: &str) -> io::Result<RollingFileAppender> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(prefix)
        .filename_suffix("log")
        .max_log_files(FILES_KEPT)
        .build(dir)
        .map_err(io::Error::other)
}

/// Deletes the flat `<prefix>.log` a pre-rotation writer left behind, so
/// switching a sink to `rolling` does not strand an unbounded file next to
/// the rotated ones. Silent when there is nothing to remove.
pub fn remove_unrotated(dir: &Path, prefix: &str) {
    let flat = dir.join(format!("{prefix}.log"));
    let _ = std::fs::remove_file(flat);
}

/// Retention for sinks that own their filenames instead of rotating -
/// one file per run, named `<prefix>-<something>.log`. Keeps the `keep`
/// most recently modified and removes the rest. Returns bytes freed.
pub fn prune_matching(dir: &Path, prefix: &str, keep: usize) -> io::Result<u64> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };

    let owned_prefix = format!("{prefix}-");
    let mut candidates: Vec<(PathBuf, Option<SystemTime>, u64)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let name = path.file_name()?.to_str()?;
            if !name.starts_with(&owned_prefix) || !name.ends_with(".log") {
                return None;
            }
            Some((path, metadata.modified().ok(), metadata.len()))
        })
        .collect();

    candidates.sort_by(|left, right| match (left.1, right.1) {
        (Some(left_time), Some(right_time)) => right_time.cmp(&left_time),
        _ => left.0.file_name().cmp(&right.0.file_name()),
    });

    let mut freed = 0;
    for (path, _, len) in candidates.iter().skip(keep) {
        if std::fs::remove_file(path).is_ok() {
            freed += len;
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn set_mtime(path: &Path, seconds: u64) -> io::Result<()> {
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)?
            .set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
    }

    #[test]
    fn prune_matching_keeps_newest_files_and_reports_bytes_freed() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let old = dir.path().join("sink-old.log");
        let mid = dir.path().join("sink-mid.log");
        let new = dir.path().join("sink-new.log");
        std::fs::write(&old, vec![0u8; 10])?;
        std::fs::write(&mid, vec![0u8; 20])?;
        std::fs::write(&new, vec![0u8; 30])?;
        set_mtime(&old, 1_000)?;
        set_mtime(&mid, 2_000)?;
        set_mtime(&new, 3_000)?;

        let freed = prune_matching(dir.path(), "sink", 1)?;

        assert_eq!(freed, 30);
        assert!(!old.exists());
        assert!(!mid.exists());
        assert!(new.exists());
        Ok(())
    }

    #[test]
    fn prune_matching_leaves_non_matching_files_alone() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let other = dir.path().join("other-a.log");
        let flat = dir.path().join("sink.log");
        std::fs::write(&other, b"keep me")?;
        std::fs::write(&flat, b"keep me too")?;

        prune_matching(dir.path(), "sink", 0)?;

        assert!(other.exists());
        assert!(flat.exists());
        Ok(())
    }

    #[test]
    fn prune_matching_returns_zero_for_missing_directory() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let freed = prune_matching(&dir.path().join("missing"), "sink", 3)?;
        assert_eq!(freed, 0);
        Ok(())
    }

    #[test]
    fn remove_unrotated_deletes_only_the_flat_file() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let flat = dir.path().join("sink.log");
        let rotated = dir.path().join("sink.2026-01-01.log");
        std::fs::write(&flat, b"stale")?;
        std::fs::write(&rotated, b"rotated")?;

        remove_unrotated(dir.path(), "sink");

        assert!(!flat.exists());
        assert!(rotated.exists());
        Ok(())
    }

    #[test]
    fn rolling_writes_under_the_expected_prefix() -> io::Result<()> {
        use std::io::Write as _;
        let dir = tempfile::tempdir()?;
        {
            let mut appender = rolling(dir.path(), "qol-log-test")?;
            appender.write_all(b"hello\n")?;
            appender.flush()?;
        }

        let mut names: Vec<String> = std::fs::read_dir(dir.path())?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        assert_eq!(names.len(), 1, "expected one rotated file, got {names:?}");
        assert!(
            names[0].starts_with("qol-log-test.") && names[0].ends_with(".log"),
            "unexpected rotated file name {names:?}"
        );
        Ok(())
    }
}
