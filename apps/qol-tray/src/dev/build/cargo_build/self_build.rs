use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

use super::super::types::BuildResult;

static LAST_ARTIFACT_COUNT: AtomicU32 = AtomicU32::new(0);

const QOL_TRAY_ID: &str = "qol-tray";

pub(super) fn build_qol_tray_self_with_progress<F>(mut on_progress: F) -> BuildResult
where
    F: FnMut(u8, String),
{
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest_path = repo_root.join("Cargo.toml");
    if let Err(error) = ensure_manifest(&manifest_path) {
        return failed_build(error);
    }

    log::info!("Building qol-tray from {}", repo_root.display());
    on_progress(2, "Preparing build".to_string());

    let mut child = match spawn_build(&repo_root, &manifest_path) {
        Ok(child) => child,
        Err(error) => return failed_build(error),
    };
    let (stdout, stderr) = take_pipes(&mut child);
    let artifact_reader = spawn_artifact_reader(stdout);
    let stderr_reader = spawn_stderr_reader(stderr);

    emit_artifact_progress(&artifact_reader.rx, &mut on_progress);

    let actual_done = artifact_reader.handle.join().unwrap_or(0);
    let combined = join_lines(stderr_reader);
    finish_build(child, actual_done, combined, &mut on_progress)
}

struct ArtifactReader {
    handle: JoinHandle<u32>,
    rx: Receiver<(u32, String)>,
}

struct LineReader {
    handle: JoinHandle<()>,
    rx: Receiver<String>,
}

struct ArtifactProgress {
    predicted: u32,
    last_percent: u8,
}

impl ArtifactProgress {
    fn new() -> Self {
        Self {
            predicted: predicted_artifact_count(),
            last_percent: 2,
        }
    }

    fn percent(&mut self, done: u32) -> u8 {
        if done > self.predicted {
            self.predicted = done + done / 4 + 1;
        }
        (((done as f32 / self.predicted as f32) * 93.0) as u8 + 2).min(95)
    }

    fn update(&mut self, done: u32) -> Option<u8> {
        let percent = self.percent(done);
        if percent <= self.last_percent {
            return None;
        }
        self.last_percent = percent;
        Some(percent)
    }
}

fn predicted_artifact_count() -> u32 {
    let previous = LAST_ARTIFACT_COUNT.load(Ordering::Relaxed);
    if previous == 0 {
        return 50;
    }
    previous
}

fn ensure_manifest(manifest_path: &Path) -> Result<(), String> {
    if manifest_path.is_file() {
        return Ok(());
    }
    Err(format!(
        "Cargo.toml not found at {}",
        manifest_path.display()
    ))
}

fn spawn_build(repo_root: &Path, manifest_path: &Path) -> Result<Child, String> {
    Command::new("cargo")
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
        .arg(manifest_path)
        .current_dir(repo_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Failed to run cargo build: {}", error))
}

fn take_pipes(child: &mut Child) -> (ChildStdout, ChildStderr) {
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    (stdout, stderr)
}

fn spawn_artifact_reader(stdout: ChildStdout) -> ArtifactReader {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || read_artifacts(stdout, tx));
    ArtifactReader { handle, rx }
}

fn spawn_stderr_reader(stderr: ChildStderr) -> LineReader {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || forward_lines(stderr, tx));
    LineReader { handle, rx }
}

fn read_artifacts(stdout: ChildStdout, tx: Sender<(u32, String)>) -> u32 {
    let mut done = 0;
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let Some(name) = artifact_name(&line) else {
            continue;
        };
        done += 1;
        let _ = tx.send((done, name));
    }
    done
}

fn artifact_name(line: &str) -> Option<String> {
    let message = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if message["reason"].as_str() != Some("compiler-artifact") {
        return None;
    }
    Some(
        message["target"]["name"]
            .as_str()
            .unwrap_or("crate")
            .to_string(),
    )
}

fn forward_lines(stderr: ChildStderr, tx: Sender<String>) {
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        let _ = tx.send(line);
    }
}

fn emit_artifact_progress<F>(rx: &Receiver<(u32, String)>, on_progress: &mut F)
where
    F: FnMut(u8, String),
{
    let mut progress = ArtifactProgress::new();
    while let Ok((done, name)) = rx.recv() {
        let Some(percent) = progress.update(done) else {
            continue;
        };
        on_progress(percent, format!("Compiling {}", name));
    }
}

fn join_lines(reader: LineReader) -> String {
    let _ = reader.handle.join();
    reader.rx.into_iter().collect::<Vec<_>>().join("\n")
}

fn finish_build<F>(
    mut child: Child,
    actual_done: u32,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    match child.wait() {
        Ok(status) if status.success() => successful_build(actual_done, combined, on_progress),
        Ok(_) => failed_status_build(combined),
        Err(error) => failed_wait_build(error),
    }
}

fn successful_build<F>(actual_done: u32, combined: String, on_progress: &mut F) -> BuildResult
where
    F: FnMut(u8, String),
{
    LAST_ARTIFACT_COUNT.store(actual_done, Ordering::Relaxed);
    on_progress(100, "Build complete".to_string());
    log::info!("qol-tray build succeeded ({} artifacts)", actual_done);
    finished_build(true, combined)
}

fn failed_status_build(combined: String) -> BuildResult {
    log::error!("qol-tray build failed\n{}", combined);
    finished_build(false, combined)
}

fn failed_wait_build(error: std::io::Error) -> BuildResult {
    let message = format!("Failed while waiting for cargo build: {}", error);
    log::error!("{}", message);
    failed_build(message)
}

fn failed_build(output: String) -> BuildResult {
    BuildResult {
        plugin_id: QOL_TRAY_ID.to_string(),
        success: false,
        output,
        skipped: false,
    }
}

fn finished_build(success: bool, output: String) -> BuildResult {
    BuildResult {
        plugin_id: QOL_TRAY_ID.to_string(),
        success,
        output,
        skipped: false,
    }
}
