use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crate::dev::adapters::traits::CargoPluginBuilder;
use super::types::BuildResult;
use crate::dev::core::progress_estimator::CargoProgressEstimator;
use crate::dev::core::progress_parser::{
    drain_console_segments, parse_console_segment, CargoProgressSnapshot,
};

static LAST_ARTIFACT_COUNT: AtomicU32 = AtomicU32::new(0);

pub(crate) struct CargoCommandPluginBuilder;

impl CargoPluginBuilder for CargoCommandPluginBuilder {
    fn build_plugin_with_progress(
        &self,
        plugin_id: &str,
        path: &Path,
        on_progress: &mut dyn FnMut(u8, String),
    ) -> BuildResult {
        build_cargo_plugin_with_progress(plugin_id, path, on_progress)
    }
}

pub fn build_qol_tray_self_with_progress<F>(mut on_progress: F) -> BuildResult
where
    F: FnMut(u8, String),
{
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest_path = repo_root.join("Cargo.toml");

    if !manifest_path.is_file() {
        return BuildResult {
            plugin_id: "qol-tray".to_string(),
            success: false,
            output: format!("Cargo.toml not found at {}", manifest_path.display()),
            skipped: false,
        };
    }

    log::info!("Building qol-tray from {}", repo_root.display());
    on_progress(2, "Preparing build".to_string());

    let mut child = match Command::new("cargo")
        .args([
            "build",
            "--bin",
            "qol-tray",
            "--features",
            "dev",
            "--message-format",
            "json",
            "--manifest-path",
        ])
        .arg(&manifest_path)
        .current_dir(&repo_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return BuildResult {
                plugin_id: "qol-tray".to_string(),
                success: false,
                output: format!("Failed to run cargo build: {}", e),
                skipped: false,
            }
        }
    };

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let (artifact_tx, artifact_rx) = std::sync::mpsc::channel::<(u32, String)>();
    let stdout_handle = std::thread::spawn(move || {
        let mut done = 0u32;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                if msg["reason"].as_str() == Some("compiler-artifact") {
                    done += 1;
                    let name = msg["target"]["name"]
                        .as_str()
                        .unwrap_or("crate")
                        .to_string();
                    let _ = artifact_tx.send((done, name));
                }
            }
        }
        done
    });

    let (text_tx, text_rx) = std::sync::mpsc::channel::<String>();
    let stderr_handle = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = text_tx.send(line);
        }
    });

    let last_count = LAST_ARTIFACT_COUNT.load(Ordering::Relaxed);
    let mut predicted = if last_count == 0 { 50u32 } else { last_count };
    let mut last_percent = 2u8;

    for (done, name) in artifact_rx {
        if done > predicted {
            predicted = done + done / 4 + 1;
        }
        let percent = ((done as f32 / predicted as f32) * 93.0) as u8 + 2;
        let percent = percent.min(95);
        if percent > last_percent {
            on_progress(percent, format!("Compiling {}", name));
            last_percent = percent;
        }
    }

    let actual_done = stdout_handle.join().unwrap_or(0);
    let _ = stderr_handle.join();
    let combined = text_rx.into_iter().collect::<Vec<_>>().join("\n");

    match child.wait() {
        Ok(status) => {
            let success = status.success();
            if success {
                LAST_ARTIFACT_COUNT.store(actual_done, Ordering::Relaxed);
                on_progress(100, "Build complete".to_string());
                log::info!("qol-tray build succeeded ({} artifacts)", actual_done);
            } else {
                log::error!("qol-tray build failed\n{}", combined);
            }
            BuildResult {
                plugin_id: "qol-tray".to_string(),
                success,
                output: combined,
                skipped: false,
            }
        }
        Err(e) => {
            let error = format!("Failed while waiting for cargo build: {}", e);
            log::error!("{}", error);
            BuildResult {
                plugin_id: "qol-tray".to_string(),
                success: false,
                output: error,
                skipped: false,
            }
        }
    }
}

pub(crate) fn build_cargo_plugin_with_progress<F>(
    plugin_id: &str,
    path: &Path,
    mut on_progress: F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    log::info!("Building linked plugin via cargo: {}", plugin_id);
    on_progress(0, "Preparing build".to_string());

    let mut child = match Command::new("cargo")
        .args(["build"])
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "80")
        .env("CARGO_TERM_COLOR", "never")
        .current_dir(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            let error = format!("Failed to run cargo build: {}", e);
            log::error!("Build error for {}: {}", plugin_id, error);
            return BuildResult {
                plugin_id: plugin_id.to_string(),
                success: false,
                output: error,
                skipped: false,
            };
        }
    };

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let (stdout_text_tx, stdout_text_rx) = std::sync::mpsc::channel::<String>();
    let stdout_handle = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                let _ = stdout_text_tx.send(line);
            }
        }
    });

    let (progress_tx, progress_rx) = std::sync::mpsc::channel::<CargoProgressSnapshot>();
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel::<String>();
    let stderr_handle = std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut buf = [0u8; 4096];
        let mut pending = String::new();

        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                    drain_console_segments(&mut pending, |raw_segment| {
                        if let Some(parsed) = parse_console_segment(raw_segment) {
                            if let Some(snapshot) = parsed.snapshot {
                                let _ = progress_tx.send(snapshot);
                            }
                            let _ = stderr_tx.send(parsed.line);
                        }
                    });
                }
                Err(_) => break,
            }
        }

        if !pending.is_empty() {
            if let Some(parsed) = parse_console_segment(&pending) {
                if let Some(snapshot) = parsed.snapshot {
                    let _ = progress_tx.send(snapshot);
                }
                let _ = stderr_tx.send(parsed.line);
            }
        }
    });

    let mut last_percent = 3u8;
    let mut last_phase = String::new();
    let mut last_emit_at = Instant::now();
    let started_at = Instant::now();
    let mut estimator = CargoProgressEstimator::default();
    let mut latest_snapshot: Option<CargoProgressSnapshot> = None;

    let mut emit_progress = |snapshot: &CargoProgressSnapshot, allow_phase_only_emit: bool| {
        let elapsed_secs = started_at.elapsed().as_secs_f64();
        let (percent, done, total) = estimator.update(snapshot.done, snapshot.total, elapsed_secs);
        let phase = if snapshot.phase.is_empty() {
            format!("{}/{}", done, total)
        } else {
            format!("{}/{} {}", done, total, snapshot.phase)
        };
        let next_percent = percent.max(last_percent);
        let percent_changed = next_percent > last_percent;
        let phase_changed = phase != last_phase;
        let can_emit_phase_only = allow_phase_only_emit
            && phase_changed
            && last_emit_at.elapsed() >= Duration::from_millis(120);

        if percent_changed || can_emit_phase_only {
            on_progress(next_percent, phase.clone());
            last_percent = next_percent;
            last_phase = phase;
            last_emit_at = Instant::now();
        }
    };

    loop {
        match progress_rx.recv_timeout(Duration::from_millis(220)) {
            Ok(snapshot) => {
                latest_snapshot = Some(snapshot);
                if let Some(latest) = latest_snapshot.as_ref() {
                    emit_progress(latest, true);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if let Some(latest) = latest_snapshot.as_ref() {
                    emit_progress(latest, false);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = stdout_handle.join();
    let _ = stderr_handle.join();
    let mut lines: Vec<String> = stdout_text_rx.into_iter().collect();
    lines.extend(stderr_rx.into_iter());
    let combined = lines.join("\n");

    match child.wait() {
        Ok(status) => {
            let success = status.success();
            if success {
                on_progress(100, "Build complete".to_string());
                log::info!("Cargo build succeeded for {}", plugin_id);
            } else {
                log::error!("Cargo build failed for {}:\n{}", plugin_id, combined);
            }
            BuildResult {
                plugin_id: plugin_id.to_string(),
                success,
                output: combined,
                skipped: false,
            }
        }
        Err(e) => {
            let error = format!("Failed while waiting for cargo build: {}", e);
            log::error!("Build error for {}: {}", plugin_id, error);
            BuildResult {
                plugin_id: plugin_id.to_string(),
                success: false,
                output: error,
                skipped: false,
            }
        }
    }
}
