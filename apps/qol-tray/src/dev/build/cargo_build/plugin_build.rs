use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::dev::core::progress_estimator::CargoProgressEstimator;
use crate::dev::core::progress_parser::{
    drain_console_segments, parse_console_segment, CargoProgressSnapshot,
};

use super::super::types::BuildResult;
use super::codesign::codesign_debug_binaries;

pub(super) fn build_cargo_plugin_with_progress<F>(
    plugin_id: &str,
    path: &Path,
    mut on_progress: F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    log::info!("Building linked plugin via cargo: {}", plugin_id);
    on_progress(0, "Preparing build".to_string());

    let mut child = match spawn_build(path) {
        Ok(child) => child,
        Err(error) => return failed_spawn(plugin_id, error),
    };
    let (stdout, stderr) = take_pipes(&mut child);
    let stdout_reader = spawn_stdout_reader(stdout);
    let stderr_reader = spawn_stderr_reader(stderr);

    ProgressDriver::default().run(&stderr_reader.progress_rx, &mut on_progress);

    let combined = join_output(stdout_reader, stderr_reader);
    finish_build(plugin_id, path, child, combined, &mut on_progress)
}

struct StdoutReader {
    handle: JoinHandle<()>,
    rx: Receiver<String>,
}

struct StderrReader {
    handle: JoinHandle<()>,
    progress_rx: Receiver<CargoProgressSnapshot>,
    line_rx: Receiver<String>,
}

struct ProgressDriver {
    estimator: CargoProgressEstimator,
    latest_snapshot: Option<CargoProgressSnapshot>,
    last_percent: u8,
    last_phase: String,
    last_emit_at: Instant,
    started_at: Instant,
}

impl Default for ProgressDriver {
    fn default() -> Self {
        Self {
            estimator: CargoProgressEstimator::default(),
            latest_snapshot: None,
            last_percent: 3,
            last_phase: String::new(),
            last_emit_at: Instant::now(),
            started_at: Instant::now(),
        }
    }
}

impl ProgressDriver {
    fn run<F>(mut self, rx: &Receiver<CargoProgressSnapshot>, on_progress: &mut F)
    where
        F: FnMut(u8, String),
    {
        loop {
            match rx.recv_timeout(Duration::from_millis(220)) {
                Ok(snapshot) => self.on_snapshot(snapshot, on_progress),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => self.on_timeout(on_progress),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn on_snapshot<F>(&mut self, snapshot: CargoProgressSnapshot, on_progress: &mut F)
    where
        F: FnMut(u8, String),
    {
        self.latest_snapshot = Some(snapshot);
        self.emit(true, on_progress);
    }

    fn on_timeout<F>(&mut self, on_progress: &mut F)
    where
        F: FnMut(u8, String),
    {
        self.emit(false, on_progress);
    }

    fn emit<F>(&mut self, allow_phase_only_emit: bool, on_progress: &mut F)
    where
        F: FnMut(u8, String),
    {
        let Some(snapshot) = self.latest_snapshot.clone() else {
            return;
        };
        let (percent, phase) = self.next_progress(&snapshot);
        if !self.should_emit(percent, &phase, allow_phase_only_emit) {
            return;
        }
        on_progress(percent, phase.clone());
        self.last_percent = percent;
        self.last_phase = phase;
        self.last_emit_at = Instant::now();
    }

    fn next_progress(&mut self, snapshot: &CargoProgressSnapshot) -> (u8, String) {
        let elapsed_secs = self.started_at.elapsed().as_secs_f64();
        let (percent, done, total) =
            self.estimator
                .update(snapshot.done, snapshot.total, elapsed_secs);
        (
            percent.max(self.last_percent),
            progress_phase(snapshot, done, total),
        )
    }

    fn should_emit(&self, percent: u8, phase: &str, allow_phase_only_emit: bool) -> bool {
        if percent > self.last_percent {
            return true;
        }
        allow_phase_only_emit
            && phase != self.last_phase
            && self.last_emit_at.elapsed() >= Duration::from_millis(120)
    }
}

fn spawn_build(path: &Path) -> Result<Child, std::io::Error> {
    Command::new("cargo")
        .arg("build")
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "80")
        .env("CARGO_TERM_COLOR", "never")
        .current_dir(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

fn take_pipes(child: &mut Child) -> (ChildStdout, ChildStderr) {
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    (stdout, stderr)
}

fn spawn_stdout_reader(stdout: ChildStdout) -> StdoutReader {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || forward_stdout(stdout, tx));
    StdoutReader { handle, rx }
}

fn spawn_stderr_reader(stderr: ChildStderr) -> StderrReader {
    let (progress_tx, progress_rx) = std::sync::mpsc::channel();
    let (line_tx, line_rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || forward_stderr(stderr, progress_tx, line_tx));
    StderrReader {
        handle,
        progress_rx,
        line_rx,
    }
}

fn forward_stdout(stdout: ChildStdout, tx: Sender<String>) {
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let _ = tx.send(line);
    }
}

fn forward_stderr(
    stderr: ChildStderr,
    progress_tx: Sender<CargoProgressSnapshot>,
    line_tx: Sender<String>,
) {
    let mut reader = BufReader::new(stderr);
    let mut buf = [0u8; 4096];
    let mut pending = String::new();

    loop {
        match reader.read(&mut buf) {
            Ok(0) => return flush_pending(&pending, &progress_tx, &line_tx),
            Ok(read) => parse_chunk(&mut pending, &buf[..read], &progress_tx, &line_tx),
            Err(_) => return,
        }
    }
}

fn parse_chunk(
    pending: &mut String,
    chunk: &[u8],
    progress_tx: &Sender<CargoProgressSnapshot>,
    line_tx: &Sender<String>,
) {
    pending.push_str(&String::from_utf8_lossy(chunk));
    drain_console_segments(pending, |segment| {
        forward_segment(segment, progress_tx, line_tx);
    });
}

fn flush_pending(
    pending: &str,
    progress_tx: &Sender<CargoProgressSnapshot>,
    line_tx: &Sender<String>,
) {
    if pending.is_empty() {
        return;
    }
    forward_segment(pending, progress_tx, line_tx);
}

fn forward_segment(
    raw_segment: &str,
    progress_tx: &Sender<CargoProgressSnapshot>,
    line_tx: &Sender<String>,
) {
    let Some(parsed) = parse_console_segment(raw_segment) else {
        return;
    };
    if let Some(snapshot) = parsed.snapshot {
        let _ = progress_tx.send(snapshot);
    }
    let _ = line_tx.send(parsed.line);
}

fn progress_phase(snapshot: &CargoProgressSnapshot, done: u32, total: u32) -> String {
    if snapshot.phase.is_empty() {
        return format!("{}/{}", done, total);
    }
    format!("{}/{} {}", done, total, snapshot.phase)
}

fn join_output(stdout_reader: StdoutReader, stderr_reader: StderrReader) -> String {
    let _ = stdout_reader.handle.join();
    let _ = stderr_reader.handle.join();
    let mut lines: Vec<String> = stdout_reader.rx.into_iter().collect();
    lines.extend(stderr_reader.line_rx);
    lines.join("\n")
}

fn finish_build<F>(
    plugin_id: &str,
    path: &Path,
    mut child: Child,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    match child.wait() {
        Ok(status) if status.success() => success_build(plugin_id, path, combined, on_progress),
        Ok(_) => failed_status_build(plugin_id, combined),
        Err(error) => failed_wait_build(plugin_id, error),
    }
}

fn success_build<F>(
    plugin_id: &str,
    path: &Path,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    codesign_debug_binaries(plugin_id, path);
    on_progress(100, "Build complete".to_string());
    log::info!("Cargo build succeeded for {}", plugin_id);
    finished_build(plugin_id, true, combined)
}

fn failed_status_build(plugin_id: &str, combined: String) -> BuildResult {
    log::error!("Cargo build failed for {}:\n{}", plugin_id, combined);
    finished_build(plugin_id, false, combined)
}

fn failed_wait_build(plugin_id: &str, error: std::io::Error) -> BuildResult {
    let message = format!("Failed while waiting for cargo build: {}", error);
    log::error!("Build error for {}: {}", plugin_id, message);
    failed_build(plugin_id, message)
}

fn failed_spawn(plugin_id: &str, error: std::io::Error) -> BuildResult {
    let message = format!("Failed to run cargo build: {}", error);
    log::error!("Build error for {}: {}", plugin_id, message);
    failed_build(plugin_id, message)
}

fn failed_build(plugin_id: &str, output: String) -> BuildResult {
    BuildResult {
        plugin_id: plugin_id.to_string(),
        success: false,
        output,
        skipped: false,
    }
}

fn finished_build(plugin_id: &str, success: bool, output: String) -> BuildResult {
    BuildResult {
        plugin_id: plugin_id.to_string(),
        success,
        output,
        skipped: false,
    }
}
