#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::Rect;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(target_os = "windows")]
pub use windows::*;

pub const CAPTURE_LOG: &str = "/tmp/record-region.log";

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct CaptureProcess {
    pub pid: u32,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct CaptureSegment {
    pub file: PathBuf,
    pub rect: Rect,
    pub offset_x: i32,
    pub offset_y: i32,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct CaptureSession {
    pub output_file: Option<PathBuf>,
    pub capture_file: Option<PathBuf>,
    pub canvas: Option<Rect>,
    pub processes: Vec<CaptureProcess>,
    #[serde(default)]
    pub segments: Vec<CaptureSegment>,
}

impl CaptureSession {
    pub fn single(pid: u32, canvas: Rect, output_file: PathBuf, capture_file: PathBuf) -> Self {
        Self {
            output_file: Some(output_file),
            capture_file: Some(capture_file),
            canvas: Some(canvas),
            processes: vec![CaptureProcess { pid }],
            segments: Vec::new(),
        }
    }

    pub fn legacy(pid: u32, output_file: Option<PathBuf>, capture_file: Option<PathBuf>) -> Self {
        Self {
            output_file,
            capture_file,
            canvas: None,
            processes: vec![CaptureProcess { pid }],
            segments: Vec::new(),
        }
    }

    pub fn pid_list(&self) -> String {
        self.processes
            .iter()
            .map(|process| process.pid.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub fn session_alive(session: &CaptureSession) -> bool {
    session
        .processes
        .iter()
        .any(|process| process_alive(process.pid))
}

pub fn session_started(session: &CaptureSession) -> bool {
    !session.processes.is_empty()
        && session
            .processes
            .iter()
            .all(|process| process_alive(process.pid))
}

#[cfg(unix)]
pub(crate) fn unix_process_alive(pid: u32) -> bool {
    if reap_child(pid) {
        return false;
    }
    let pid = pid as libc::pid_t;
    pid > 0 && unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(unix)]
pub(crate) fn unix_signal_process(pid: u32, signal: i32) -> anyhow::Result<()> {
    let pid = pid as libc::pid_t;
    if pid <= 0 {
        return Err(anyhow::anyhow!("invalid process pid {}", pid));
    }
    if unsafe { libc::kill(pid, signal) } == 0 || !unix_process_alive(pid as u32) {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "failed to send signal {} to process pid {}",
        signal,
        pid
    ))
}

#[cfg(unix)]
fn reap_child(pid: u32) -> bool {
    let pid = pid as libc::pid_t;
    if pid <= 0 {
        return false;
    }
    loop {
        let mut status = 0;
        let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if reaped == pid {
            return true;
        }
        if reaped == 0 {
            return false;
        }
        if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return false;
    }
}
