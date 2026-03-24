use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::daemon::{DaemonEvent, EventBus};

pub(super) fn build_dev_binary(repo_path: &Path, events: Arc<EventBus>) -> Result<PathBuf> {
    events.send(DaemonEvent::ModeSwitchProgress {
        percent: 0,
        phase: "Starting dev build".into(),
    });

    let mut child = Command::new("cargo")
        .args(["build", "--features", "dev", "--bin", "qol-tray"])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn cargo build")?;

    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);
    for line in reader.lines().map_while(Result::ok) {
        if let Some(phase) = parse_cargo_phase(&line) {
            events.send(DaemonEvent::ModeSwitchProgress { percent: 50, phase });
        }
    }

    let status = child.wait().context("cargo build failed to complete")?;
    if !status.success() {
        anyhow::bail!("cargo build exited with {}", status);
    }

    events.send(DaemonEvent::ModeSwitchProgress {
        percent: 100,
        phase: "Build complete".into(),
    });

    let binary = repo_path.join("target/debug/qol-tray");
    if !binary.is_file() {
        anyhow::bail!("Built binary not found at {}", binary.display());
    }
    Ok(binary)
}

fn parse_cargo_phase(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with("Compiling")
        || trimmed.starts_with("Linking")
        || trimmed.starts_with("Downloading")
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}
