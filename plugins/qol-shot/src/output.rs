use anyhow::{anyhow, Context, Result};
use chrono::Local;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn recording_output_file_path(format: &str) -> Result<PathBuf> {
    let home = env::var("HOME").context("HOME is not set")?;
    let mut videos = PathBuf::from(home);
    videos.push("Videos");
    fs::create_dir_all(&videos).context("failed to create output directory")?;
    let timestamp = Local::now().format("%F_%H-%M-%S").to_string();
    videos.push(format!("recording-{}.{}", timestamp, format));
    Ok(videos)
}

pub(crate) fn screenshot_output_file_path() -> Result<PathBuf> {
    let pictures = screenshot_dir()?;
    fs::create_dir_all(&pictures).context("failed to create screenshot output directory")?;
    let timestamp = Local::now().format("%F_%H-%M-%S_%9f").to_string();
    Ok(unique_screenshot_path(
        &pictures,
        &timestamp,
        std::process::id(),
    ))
}

pub(crate) fn latest_screenshot() -> Result<PathBuf> {
    let pictures = screenshot_dir()?;
    fs::read_dir(&pictures)
        .with_context(|| format!("failed to read {}", pictures.display()))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_screenshot_file(path))
        .filter_map(|path| Some((path.metadata().ok()?.modified().ok()?, path)))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
        .ok_or_else(|| anyhow!("no screenshots found in {}", pictures.display()))
}

fn screenshot_dir() -> Result<PathBuf> {
    let home = env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Pictures"))
}

fn is_screenshot_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("screenshot-") && name.ends_with(".png"))
}

fn unique_screenshot_path(directory: &Path, timestamp: &str, process_id: u32) -> PathBuf {
    for attempt in 0..1000 {
        let candidate = directory.join(screenshot_file_name(timestamp, process_id, attempt));
        if candidate.symlink_metadata().is_err() {
            return candidate;
        }
    }

    directory.join(screenshot_file_name(timestamp, process_id, 1000))
}

fn screenshot_file_name(timestamp: &str, process_id: u32, attempt: u16) -> String {
    let suffix = if attempt == 0 {
        String::new()
    } else {
        format!("-{attempt}")
    };
    format!("screenshot-{timestamp}-{process_id}{suffix}.png")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_file_names_include_subseconds_and_collision_suffix() {
        let first = screenshot_file_name("2026-06-21_11-51-59_123456789", 42, 0);
        let second = screenshot_file_name("2026-06-21_11-51-59_123456789", 42, 1);

        assert_eq!(first, "screenshot-2026-06-21_11-51-59_123456789-42.png");
        assert_eq!(second, "screenshot-2026-06-21_11-51-59_123456789-42-1.png");
    }

    #[test]
    fn is_screenshot_file_matches_capture_naming() {
        let cases = [
            ("screenshot-2026-06-21_11-51-59_123-42.png", true),
            ("screenshot-2026-06-21_11-51-59_123-42-1.png", true),
            ("recording-2026-06-21.mov", false),
            ("screenshot-foo.jpg", false),
            ("note.png", false),
        ];
        for (name, expected) in cases {
            assert_eq!(
                is_screenshot_file(Path::new(name)),
                expected,
                "name: {name}"
            );
        }
    }
}
