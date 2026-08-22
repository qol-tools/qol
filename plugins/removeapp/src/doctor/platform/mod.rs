use std::io::ErrorKind;
use std::path::PathBuf;

use serde_json::{json, Value};

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::inspect;
#[cfg(target_os = "linux")]
pub(super) use linux::inspect;
#[cfg(target_os = "macos")]
pub(super) use macos::inspect;
#[cfg(target_os = "windows")]
pub(super) use windows::inspect;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DirectoryState {
    Directory,
    Missing,
    WrongType,
    Unreadable(ErrorKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirectoryInspection {
    pub(super) path: PathBuf,
    pub(super) state: DirectoryState,
}

impl DirectoryInspection {
    pub(super) fn details(&self) -> Value {
        let (state, error_kind) = match self.state {
            DirectoryState::Directory => ("directory", None),
            DirectoryState::Missing => ("missing", None),
            DirectoryState::WrongType => ("wrong_type", None),
            DirectoryState::Unreadable(kind) => ("unreadable", Some(format!("{kind:?}"))),
        };
        json!({
            "path": self.path,
            "state": state,
            "error_kind": error_kind,
        })
    }
}

pub(super) struct PlatformInspection {
    pub(super) name: &'static str,
    pub(super) supported: bool,
    pub(super) inventory_roots: Vec<DirectoryInspection>,
    pub(super) trash: Option<DirectoryInspection>,
    pub(super) trash_creation_anchor: Option<DirectoryInspection>,
}
