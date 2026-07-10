use anyhow::{Context, Result};
use qol_headless::DoctorCheckResult;
use std::env;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn show_notification(title: &str, message: &str, timeout_ms: u32) {
    let _ = Command::new("notify-send")
        .args([
            "-u",
            "normal",
            "-t",
            &timeout_ms.to_string(),
            title,
            message,
        ])
        .status();
}

pub fn open_url(url: &str) -> Result<()> {
    open::that(url).context("failed to open URL")
}

pub fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "platform_supported",
        "Linux capture is supported through ffmpeg/x11grab.",
    )
}

pub fn required_binaries_check() -> DoctorCheckResult {
    let required = ["ffmpeg", "xrandr", "xdpyinfo"];
    let missing = required
        .iter()
        .copied()
        .filter(|name| resolve_command(name).is_none())
        .collect::<Vec<_>>();

    if missing.is_empty() {
        return DoctorCheckResult::ok(
            "required_binaries",
            "Required Linux capture tools are available.",
        );
    }

    DoctorCheckResult::fail(
        "required_binaries",
        format!("Missing required binaries: {}.", missing.join(", ")),
    )
    .with_fix("Install ffmpeg, xrandr, and xdpyinfo.")
}

pub(super) fn resolve_command(command: &str) -> Option<PathBuf> {
    command_search_dirs()
        .into_iter()
        .map(|dir| dir.join(command))
        .find(|path| is_executable_file(path))
}

fn command_search_dirs() -> Vec<PathBuf> {
    let mut dirs = env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    dirs.extend([
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    dirs
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}
