use std::io::{BufRead, BufReader, Read};
use std::process::{ChildStderr, ChildStdout};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

use crate::dev::core::progress_parser::{
    drain_console_segments, parse_console_segment, CargoProgressSnapshot,
};

pub(super) struct OutputReaders {
    stdout: LineReader,
    stderr: StderrReader,
}

struct LineReader {
    handle: JoinHandle<()>,
    rx: Receiver<String>,
}

struct StderrReader {
    handle: JoinHandle<()>,
    progress_rx: Receiver<CargoProgressSnapshot>,
    line_rx: Receiver<String>,
}

pub(super) fn spawn_output_readers(stdout: ChildStdout, stderr: ChildStderr) -> OutputReaders {
    OutputReaders {
        stdout: spawn_stdout_reader(stdout),
        stderr: spawn_stderr_reader(stderr),
    }
}

impl OutputReaders {
    pub(super) fn progress_rx(&self) -> &Receiver<CargoProgressSnapshot> {
        &self.stderr.progress_rx
    }

    pub(super) fn join_output(self) -> String {
        let _ = self.stdout.handle.join();
        let _ = self.stderr.handle.join();
        let mut lines: Vec<String> = self.stdout.rx.into_iter().collect();
        lines.extend(self.stderr.line_rx);
        lines.join("\n")
    }
}

fn spawn_stdout_reader(stdout: ChildStdout) -> LineReader {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || forward_stdout(stdout, tx));
    LineReader { handle, rx }
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
