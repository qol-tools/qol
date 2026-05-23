use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn archive_path(config_dir: &Path, migration_name: &str) -> Result<PathBuf> {
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let safe = sanitize(migration_name);
    Ok(config_dir.join("archive").join(format!("{safe}-{ts}")))
}

pub(crate) fn move_into_archive(src: &Path, archive_dir: &Path) -> Result<PathBuf> {
    let name = src
        .file_name()
        .with_context(|| format!("source has no file name: {}", src.display()))?;
    let dst = archive_dir.join(name);
    match std::fs::rename(src, &dst) {
        Ok(()) => Ok(dst),
        Err(_) => {
            if src.is_dir() {
                copy_dir_all(src, &dst).context("copying dir to archive")?;
                std::fs::remove_dir_all(src).context("removing source dir after archive copy")?;
            } else {
                std::fs::copy(src, &dst).context("copying file to archive")?;
                std::fs::remove_file(src).context("removing source file after archive copy")?;
            }
            Ok(dst)
        }
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_path_includes_sanitized_name_and_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = archive_path(dir.path(), "v3.15-to-v3.16").unwrap();
        let segment = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(segment.starts_with("v3.15-to-v3.16-"), "got {segment}");
        assert_eq!(path.parent().unwrap(), dir.path().join("archive"));
    }

    #[test]
    fn archive_path_replaces_forbidden_chars() {
        let dir = tempfile::tempdir().unwrap();
        let path = archive_path(dir.path(), "v3.15 → v3.16").unwrap();
        let segment = path.file_name().unwrap().to_string_lossy().into_owned();
        for c in segment.chars() {
            assert!(
                c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'),
                "unexpected char {c:?} in {segment}"
            );
        }
    }

    #[test]
    fn move_into_archive_moves_file_and_dir() {
        let work = tempfile::tempdir().unwrap();
        let archive = work.path().join("archive");
        std::fs::create_dir_all(&archive).unwrap();

        let file = work.path().join("a.json");
        std::fs::write(&file, b"{}").unwrap();
        let dir = work.path().join("b");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested").join("c.json"), b"[]").unwrap();

        let cases = [
            (file.clone(), archive.join("a.json")),
            (dir.clone(), archive.join("b")),
        ];
        for (src, expected_dst) in cases {
            let dst = move_into_archive(&src, &archive).unwrap();
            assert_eq!(dst, expected_dst, "src: {}", src.display());
            assert!(!src.exists(), "src should be gone: {}", src.display());
            assert!(dst.exists(), "dst should exist: {}", dst.display());
        }
        assert!(archive.join("b").join("nested").join("c.json").exists());
    }
}
