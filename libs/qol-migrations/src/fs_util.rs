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

pub(crate) fn current_os_subdir() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

pub(crate) fn is_safe_path_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

pub(crate) fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub(crate) fn legacy_sidecar_path(src: &Path) -> PathBuf {
    let mut name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(".legacy");
    src.with_file_name(name)
}

pub(crate) fn list_profile_dirs(profile_dir: &Path) -> Result<Vec<PathBuf>> {
    if !profile_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(profile_dir)
        .with_context(|| format!("reading {}", profile_dir.display()))?
    {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("manifest.json").is_file() {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

pub(crate) fn list_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

/// Every plugin-configs dir on disk for a profile: core, device, and each os
/// bucket. All buckets are walked so a synced foreign-OS profile is migrated
/// too, not just the running OS.
pub(crate) fn plugin_config_dirs(profile_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        profile_dir.join("core").join("plugin-configs"),
        profile_dir.join("device").join("plugin-configs"),
    ];
    for bucket in list_subdirs(&profile_dir.join("os")) {
        dirs.push(bucket.join("plugin-configs"));
    }
    dirs
}

pub(crate) fn hotkey_files(profile_dir: &Path) -> Vec<PathBuf> {
    list_subdirs(&profile_dir.join("os"))
        .into_iter()
        .map(|bucket| bucket.join("hotkeys.json"))
        .filter(|p| p.is_file())
        .collect()
}

pub(crate) fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<()> {
    let serialized = serde_json::to_string_pretty(value).context("serializing migrated json")?;
    qol_fs::atomic_write(path, serialized.as_bytes())
        .with_context(|| format!("writing migrated json to {}", path.display()))
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
