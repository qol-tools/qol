use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const APPLIED_DIR: &str = "migrations/applied";

fn applied_dir(config_dir: &Path) -> PathBuf {
    config_dir.join(APPLIED_DIR)
}

fn done_path(config_dir: &Path, name: &str) -> PathBuf {
    applied_dir(config_dir).join(format!("{name}.done"))
}

pub(crate) fn is_done(config_dir: &Path, name: &str) -> bool {
    done_path(config_dir, name).is_file()
}

pub(crate) fn write_done(config_dir: &Path, name: &str) -> Result<()> {
    let dir = applied_dir(config_dir);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating journal dir {}", dir.display()))?;
    let final_path = done_path(config_dir, name);
    let tmp_path = dir.join(format!("{name}.done.tmp"));
    std::fs::write(&tmp_path, chrono::Utc::now().to_rfc3339())
        .with_context(|| format!("writing journal tmp {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "renaming journal {} -> {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_done_then_is_done_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let name = "v3.15-to-v3.16";

        assert!(!is_done(dir.path(), name), "fresh dir has no journal entry");

        write_done(dir.path(), name).unwrap();
        assert!(is_done(dir.path(), name), "journaled after first write");

        write_done(dir.path(), name).unwrap();
        assert!(
            is_done(dir.path(), name),
            "journal remains after a second write"
        );
    }

    #[test]
    fn write_done_lands_under_applied_dir() {
        let dir = tempfile::tempdir().unwrap();
        let name = "alpha";

        write_done(dir.path(), name).unwrap();

        let expected = dir.path().join(APPLIED_DIR).join(format!("{name}.done"));
        assert!(expected.is_file(), "expected file at {}", expected.display());
    }

    #[test]
    fn write_done_does_not_leave_tmp_behind() {
        let dir = tempfile::tempdir().unwrap();
        let name = "beta";

        write_done(dir.path(), name).unwrap();

        let tmp = dir
            .path()
            .join(APPLIED_DIR)
            .join(format!("{name}.done.tmp"));
        assert!(!tmp.exists(), "tmp file should be renamed away");
    }

    #[test]
    fn distinct_names_journal_independently() {
        let dir = tempfile::tempdir().unwrap();
        write_done(dir.path(), "a").unwrap();
        assert!(is_done(dir.path(), "a"));
        assert!(!is_done(dir.path(), "b"));
    }
}
