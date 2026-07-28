use std::path::PathBuf;
use std::process::Command;

pub(super) enum RevealPlan {
    Open(PathBuf),
    Command(Command),
}

impl From<PathBuf> for RevealPlan {
    fn from(path: PathBuf) -> Self {
        Self::Open(path)
    }
}

impl From<Command> for RevealPlan {
    fn from(command: Command) -> Self {
        Self::Command(command)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

pub(super) fn reveal_plan(path: &std::path::Path) -> RevealPlan {
    imp::reveal_plan(path)
}
