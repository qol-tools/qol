use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio_stream::StreamExt;

use crate::daemon::{DaemonEvent, EventBus};

pub(crate) async fn download_asset(url: &str, dest: &Path, events: &EventBus) -> Result<()> {
    let request = crate::features::plugin_store::github::build_github_request(
        &reqwest::Client::new(),
        url,
        None,
    );
    let response = crate::features::plugin_store::github::send_checked(request).await?;
    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut file = std::fs::File::create(dest)?;
    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        let percent = total
            .map(|t| ((downloaded * 100) / t).min(100) as u8)
            .unwrap_or(0);
        if percent != last_percent {
            events.send(DaemonEvent::UpdateProgress { percent });
            last_percent = percent;
        }
    }
    file.sync_all()?;
    Ok(())
}

pub(crate) fn extract_tar_gz(archive: &Path, binary_name: &str) -> Result<PathBuf> {
    let tar_gz = std::fs::File::open(archive)?;
    let tar = flate2::read::GzDecoder::new(tar_gz);
    let mut reader = tar::Archive::new(tar);

    let extract_dir = archive.with_extension("extracted");
    if extract_dir.exists() {
        let _ = std::fs::remove_dir_all(&extract_dir);
    }
    std::fs::create_dir_all(&extract_dir)?;
    reader.unpack(&extract_dir)?;

    for entry in walkdir::WalkDir::new(&extract_dir).max_depth(2) {
        let entry = entry?;
        if entry.file_type().is_file() && entry.file_name().to_string_lossy() == binary_name {
            return Ok(entry.into_path());
        }
    }
    anyhow::bail!("Binary '{binary_name}' not found in archive")
}

pub(crate) fn atomic_replace(source: &Path, target: &Path) -> Result<()> {
    let staged = target.with_extension("new");
    let result = atomic_replace_inner(source, target, &staged);
    if result.is_err() && staged.exists() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

fn atomic_replace_inner(source: &Path, target: &Path, staged: &Path) -> Result<()> {
    if staged.exists() {
        let _ = std::fs::remove_file(staged);
    }
    std::fs::copy(source, staged).with_context(|| {
        format!(
            "Failed to stage {} to {}",
            source.display(),
            staged.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(staged)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(staged, perms)?;
    }

    std::fs::rename(staged, target)
        .with_context(|| format!("Failed to replace {}", target.display()))?;
    Ok(())
}

pub(crate) fn cleanup_archive(archive: &Path) {
    let _ = std::fs::remove_file(archive);
    let extract_dir = archive.with_extension("extracted");
    let _ = std::fs::remove_dir_all(&extract_dir);
}

pub(crate) fn arch_suffix() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64"
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    compile_error!("unsupported architecture")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_tar_gz(dir: &Path, bundle_name: &str, binary_name: &str) -> PathBuf {
        let archive_path = dir.join("test.tar.gz");
        let buf = Vec::new();
        let encoder = flate2::write::GzEncoder::new(buf, flate2::Compression::fast());
        let mut tar = tar::Builder::new(encoder);

        let content = b"fake binary content";
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("{bundle_name}/{binary_name}"),
            &content[..],
        )
        .unwrap();

        let encoder = tar.into_inner().unwrap();
        let bytes = encoder.finish().unwrap();
        std::fs::write(&archive_path, bytes).unwrap();
        archive_path
    }

    #[test]
    fn extract_finds_binary_at_depth_one() {
        let dir = tempfile::tempdir().unwrap();
        let archive = create_test_tar_gz(dir.path(), "qol-tray-linux-x86_64", "qol-tray");
        let result = extract_tar_gz(&archive, "qol-tray");
        assert!(result.is_ok(), "{}", result.unwrap_err());
        let path = result.unwrap();
        assert_eq!(path.file_name().unwrap(), "qol-tray");
        assert_eq!(std::fs::read(&path).unwrap(), b"fake binary content");
    }

    #[test]
    fn extract_fails_when_binary_missing() {
        let dir = tempfile::tempdir().unwrap();
        let archive = create_test_tar_gz(dir.path(), "bundle", "other-binary");
        let result = extract_tar_gz(&archive, "qol-tray");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn atomic_replace_swaps_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        std::fs::write(&source, b"new content").unwrap();
        std::fs::write(&target, b"old content").unwrap();
        atomic_replace(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new content");
        assert!(!dir.path().join("target.new").exists());
    }

    #[test]
    fn atomic_replace_creates_target_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        std::fs::write(&source, b"content").unwrap();
        atomic_replace(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"content");
    }
}
