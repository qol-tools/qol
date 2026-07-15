use anyhow::{Context, Result};
use qol_headless::DoctorCheckResult;
use std::env;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

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

pub fn show_saved_notification(
    title: &str,
    message: &str,
    timeout_ms: u32,
    target: crate::completion::RevealTarget,
) {
    let mut command = saved_notification_command(title, message, timeout_ms);
    let title = title.to_string();
    let message = message.to_string();
    thread::spawn(move || {
        let Ok(output) = command.output() else {
            show_notification(&title, &message, timeout_ms);
            return;
        };
        if !output.status.success() {
            show_notification(&title, &message, timeout_ms);
            return;
        }
        let action = notification_action(&output.stdout);
        let clicked = action.is_some();
        qol_runtime::probe!(
            "SHOT_NOTIFICATION_ACTION",
            "kind=open-folder clicked={clicked} file={}",
            target
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("capture")
        );
        if !clicked {
            return;
        }
        if let Err(error) = target.open(crate::completion::RevealSource::Notification) {
            eprintln!("[qol-shot] notification folder reveal failed: {error:#}");
        }
    });
}

fn saved_notification_command(title: &str, message: &str, timeout_ms: u32) -> Command {
    let mut command = Command::new("notify-send");
    command
        .args(["-u", "normal", "-t"])
        .arg(timeout_ms.to_string())
        .args(["-a", "QoL Shot"])
        .args(["-A", "default=Open Folder"])
        .args(["-A", "open-folder=Open Folder"])
        .arg(title)
        .arg(message)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn notification_action(output: &[u8]) -> Option<&'static str> {
    match String::from_utf8_lossy(output).trim() {
        "default" => Some("default"),
        "open-folder" => Some("open-folder"),
        _ => None,
    }
}

pub fn open_url(url: &str) -> Result<()> {
    qol_apps::desktop_integration::open_with_default_app(url).context("failed to open URL")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn saved_notification_exposes_default_and_button_actions() {
        let command = saved_notification_command("Recording saved", "video file", 8_000);
        let args = command.get_args().collect::<Vec<_>>();

        assert_eq!(command.get_program(), OsStr::new("notify-send"));
        assert!(args
            .windows(2)
            .any(|pair| { pair == [OsStr::new("-A"), OsStr::new("default=Open Folder")] }));
        assert!(args
            .windows(2)
            .any(|pair| { pair == [OsStr::new("-A"), OsStr::new("open-folder=Open Folder")] }));
        assert_eq!(args[args.len() - 2], OsStr::new("Recording saved"));
        assert_eq!(args[args.len() - 1], OsStr::new("video file"));
    }

    #[test]
    fn notification_action_accepts_body_and_button_clicks_only() {
        let cases = [
            (b"default\n".as_slice(), Some("default")),
            (b"open-folder\n".as_slice(), Some("open-folder")),
            (b"closed\n".as_slice(), None),
            (b"".as_slice(), None),
        ];

        for (output, expected) in cases {
            assert_eq!(notification_action(output), expected);
        }
    }
}
