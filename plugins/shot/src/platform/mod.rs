#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod recorder;
#[cfg(unix)]
mod unix;
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

macro_rules! record_region_base {
    () => {
        "record-region"
    };
}
pub const RECORD_REGION_BASE: &str = record_region_base!();

const CAPTURE_STATE_NAME: &str = concat!(record_region_base!(), ".pid");
const CAPTURE_LOG_NAME: &str = concat!(record_region_base!(), ".log");

pub fn runtime_dir() -> PathBuf {
    qol_config::runtime_dir()
        .map(|dir| dir.join("shot"))
        .unwrap_or_else(|| std::env::temp_dir().join("qol-shot"))
}

pub fn ensure_runtime_dir() -> std::io::Result<PathBuf> {
    let dir = runtime_dir();
    qol_fs::create_private_dir(&dir)?;
    let metadata = std::fs::symlink_metadata(&dir)?;
    if !owned_by_current_user(&metadata) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} is owned by another user", dir.display()),
        ));
    }
    Ok(dir)
}

pub fn capture_state_path() -> PathBuf {
    runtime_dir().join(CAPTURE_STATE_NAME)
}

pub fn capture_log_path() -> PathBuf {
    runtime_dir().join(CAPTURE_LOG_NAME)
}

pub fn open_capture_log(truncate: bool) -> std::io::Result<std::fs::File> {
    let dir = ensure_runtime_dir()?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true);
    if truncate {
        options.truncate(true);
    } else {
        options.append(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(dir.join(CAPTURE_LOG_NAME))
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioDevice {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SavedRecording {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct CaptureProcess {
    pub pid: u32,
    #[serde(default)]
    pub identity: Option<String>,
}

impl CaptureProcess {
    pub fn spawned(pid: u32) -> Self {
        Self {
            pid,
            identity: qol_process::process_identity(pid).ok(),
        }
    }

    pub fn alive(&self) -> bool {
        self.identity
            .as_deref()
            .is_some_and(|identity| qol_process::process_identity_matches(self.pid, identity))
            && process_alive(self.pid)
    }
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
    pub fn legacy(pid: u32, output_file: Option<PathBuf>, capture_file: Option<PathBuf>) -> Self {
        Self {
            output_file,
            capture_file,
            canvas: None,
            processes: vec![CaptureProcess {
                pid,
                identity: None,
            }],
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
    session.processes.iter().any(CaptureProcess::alive)
}

pub fn session_started(session: &CaptureSession) -> bool {
    !session.processes.is_empty() && session.processes.iter().all(CaptureProcess::alive)
}
