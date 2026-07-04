use std::io::{BufRead, BufReader};
use std::process::{ChildStderr, ChildStdout};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

pub(super) struct ArtifactReaders {
    artifacts: ArtifactReader,
    stderr: LineReader,
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
            predicted: super::predicted_artifact_count(),
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

pub(super) fn spawn_readers(stdout: ChildStdout, stderr: ChildStderr) -> ArtifactReaders {
    ArtifactReaders {
        artifacts: spawn_artifact_reader(stdout),
        stderr: spawn_stderr_reader(stderr),
    }
}

impl ArtifactReaders {
    pub(super) fn emit_progress<F>(&self, on_progress: &mut F)
    where
        F: FnMut(u8, String),
    {
        let mut progress = ArtifactProgress::new();
        while let Ok((done, name)) = self.artifacts.rx.recv() {
            let Some(percent) = progress.update(done) else {
                continue;
            };
            on_progress(percent, format!("Compiling {}", name));
        }
    }

    pub(super) fn join(self) -> (u32, String) {
        let actual_done = self.artifacts.handle.join().unwrap_or(0);
        let _ = self.stderr.handle.join();
        let combined = self.stderr.rx.into_iter().collect::<Vec<_>>().join("\n");
        (actual_done, combined)
    }
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
