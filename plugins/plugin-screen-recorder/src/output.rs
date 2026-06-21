use anyhow::{Context, Result};
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
    let home = env::var("HOME").context("HOME is not set")?;
    let mut pictures = PathBuf::from(home);
    pictures.push("Pictures");
    fs::create_dir_all(&pictures).context("failed to create screenshot output directory")?;
    let timestamp = Local::now().format("%F_%H-%M-%S_%9f").to_string();
    Ok(unique_screenshot_path(
        &pictures,
        &timestamp,
        std::process::id(),
    ))
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
}
