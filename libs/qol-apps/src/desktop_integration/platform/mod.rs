#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use std::path::PathBuf;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::process::Command;

pub(super) enum RevealPlan {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Open(PathBuf),
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    Command(Command),
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod other;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use other as imp;
#[cfg(target_os = "windows")]
use windows as imp;

pub(super) fn reveal_plan(path: &std::path::Path) -> RevealPlan {
    imp::reveal_plan(path)
}
