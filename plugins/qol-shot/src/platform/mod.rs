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
