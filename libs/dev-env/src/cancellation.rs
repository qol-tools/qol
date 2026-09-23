use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::resources::validate_run_id;

const CANCELLATION_DIR: &str = "dev-env-cancellations";

#[must_use = "keep the inbox alive while the environment launch is active"]
pub struct CancellationInbox {
    path: PathBuf,
}

impl CancellationInbox {
    pub fn for_run(run_id: &str) -> Result<Self> {
        Self::in_root(cancellation_root(), run_id)
    }

    pub fn is_requested(&self) -> Result<bool> {
        match self.path.symlink_metadata() {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to inspect cancellation request {}",
                    self.path.display()
                )
            }),
        }
    }

    fn in_root(root: PathBuf, run_id: &str) -> Result<Self> {
        validate_run_id(run_id)?;
        Ok(Self {
            path: root.join(format!("{run_id}.request")),
        })
    }
}

impl Drop for CancellationInbox {
    fn drop(&mut self) {
        let _ = remove_request(&self.path);
    }
}

pub fn request_cancellation(run_id: &str) -> Result<PathBuf> {
    request_in(&cancellation_root(), run_id)
}

pub fn clear_cancellation_request(run_id: &str) -> Result<()> {
    validate_run_id(run_id)?;
    remove_request(&cancellation_root().join(format!("{run_id}.request")))
}

fn request_in(root: &Path, run_id: &str) -> Result<PathBuf> {
    validate_run_id(run_id)?;
    let path = root.join(format!("{run_id}.request"));
    qol_fs::atomic_write(&path, b"cancel\n")
        .with_context(|| format!("failed to request cancellation at {}", path.display()))?;
    Ok(path)
}

fn remove_request(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to clear cancellation request {}", path.display())),
    }
}

fn cancellation_root() -> PathBuf {
    qol_config::data_subdir("runtime")
        .unwrap_or_else(std::env::temp_dir)
        .join(CANCELLATION_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_visible_until_the_inbox_is_dropped() {
        let temp = tempfile::tempdir().unwrap();
        let inbox = CancellationInbox::in_root(temp.path().to_path_buf(), "batch-1").unwrap();
        assert!(!inbox.is_requested().unwrap());

        let path = request_in(temp.path(), "batch-1").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "cancel\n");
        assert!(inbox.is_requested().unwrap());

        drop(inbox);
        assert!(!path.exists());
    }

    #[test]
    fn a_request_created_before_the_inbox_is_not_lost() {
        let temp = tempfile::tempdir().unwrap();
        let path = request_in(temp.path(), "batch-early").unwrap();
        let inbox = CancellationInbox::in_root(temp.path().to_path_buf(), "batch-early").unwrap();

        assert!(inbox.is_requested().unwrap());
        drop(inbox);
        assert!(!path.exists());
    }

    #[test]
    fn ids_cannot_escape_the_runtime_directory() {
        let temp = tempfile::tempdir().unwrap();
        for invalid in ["", "../batch", "batch/one", "batch one"] {
            assert!(request_in(temp.path(), invalid).is_err(), "{invalid:?}");
            assert!(
                CancellationInbox::in_root(temp.path().to_path_buf(), invalid).is_err(),
                "{invalid:?}"
            );
        }
    }
}
