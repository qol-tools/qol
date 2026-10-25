use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
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

#[cfg(not(target_os = "windows"))]
mod commands;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod handlers;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "linux"))]
mod unsupported;
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

pub(super) fn open(target: &OsStr) -> io::Result<()> {
    imp::open(target)
}

pub(super) fn open_checked(target: &OsStr) -> io::Result<()> {
    imp::open_checked(target)
}

pub(super) fn reveal_plan(path: &std::path::Path) -> RevealPlan {
    imp::reveal_plan(path)
}

pub(super) fn open_with_supported() -> bool {
    imp::open_with_supported()
}

pub(super) fn file_type(path: &Path) -> io::Result<String> {
    imp::file_type(path)
}

pub(super) fn open_handlers(path: &Path) -> io::Result<Vec<super::OpenHandler>> {
    imp::open_handlers(path)
}

pub(super) fn open_with_handler(handler: &str, paths: &[PathBuf]) -> io::Result<()> {
    imp::open_with_handler(handler, paths)
}

pub(super) fn terminal_commands(folder: &Path) -> Vec<Command> {
    imp::terminal_commands(folder)
}
