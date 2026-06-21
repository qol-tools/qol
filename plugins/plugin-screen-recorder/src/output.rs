use anyhow::{Context, Result};
use chrono::Local;
use std::env;
use std::fs;
use std::path::PathBuf;

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
    let timestamp = Local::now().format("%F_%H-%M-%S").to_string();
    pictures.push(format!("screenshot-{timestamp}.png"));
    Ok(pictures)
}
