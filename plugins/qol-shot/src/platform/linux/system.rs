use crate::platform::AudioDevice;
use anyhow::{Context, Result};
use qol_headless::DoctorCheckResult;
use std::env;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use x11rb::connection::Connection;

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
    target: crate::capture::completion::RevealTarget,
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
        if let Err(error) = target.open("notification") {
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

pub fn permissions_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "permissions",
        "Linux X11 capture requires no separate OS permission grant.",
    )
    .with_details(serde_json::json!({
        "platform": "linux",
        "authorization": "x11_session",
        "prompted": false,
        "capture_attempted": false,
    }))
}

pub fn external_services_check() -> DoctorCheckResult {
    let display = std::env::var_os("DISPLAY")
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string_lossy().into_owned());
    let observation = x11rb::connect(None)
        .map(|(connection, _)| connection.setup().roots.len())
        .map_err(|error| error.to_string());
    external_services_result(display, observation)
}

fn external_services_result(
    display: Option<String>,
    observation: Result<usize, String>,
) -> DoctorCheckResult {
    let details = serde_json::json!({
        "platform": "linux",
        "service": "x11",
        "display": display,
        "screen_count": observation.as_ref().ok(),
        "connected": observation.is_ok(),
        "capture_attempted": false,
    });
    match observation {
        Ok(count) => DoctorCheckResult::ok(
            "external_services",
            format!("The X11 display service responded with {count} screen(s)."),
        )
        .with_details(details),
        Err(error) => DoctorCheckResult::fail(
            "external_services",
            format!("The X11 display service is unavailable: {error}"),
        )
        .with_fix("Run qol-shot in an authorized X11 or XWayland graphical session.")
        .with_details(details),
    }
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

pub fn list_audio_sources() -> Vec<AudioDevice> {
    pactl_devices("sources")
        .into_iter()
        .filter(|device| !device.value.ends_with(".monitor"))
        .collect()
}

pub fn list_audio_sinks() -> Vec<AudioDevice> {
    pactl_devices("sinks")
}

fn pactl_devices(kind: &str) -> Vec<AudioDevice> {
    let output = Command::new("pactl")
        .args(["--format=json", "list", kind])
        .stdin(Stdio::null())
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_pactl_devices(&String::from_utf8_lossy(&output.stdout))
}

fn parse_pactl_devices(raw: &str) -> Vec<AudioDevice> {
    let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let value = entry.get("name")?.as_str()?.to_string();
            let label = entry
                .get("description")
                .and_then(|description| description.as_str())
                .unwrap_or(value.as_str())
                .to_string();
            Some(AudioDevice { value, label })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn pactl_device_parsing_maps_names_and_descriptions() {
        let raw = r#"[
            {"index": 1, "name": "alsa_input.foo", "description": "Built-in Microphone"},
            {"index": 2, "name": "alsa_output.bar.monitor"},
            {"index": 3, "description": "nameless is skipped"}
        ]"#;
        let devices = parse_pactl_devices(raw);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].value, "alsa_input.foo");
        assert_eq!(devices[0].label, "Built-in Microphone");
        assert_eq!(devices[1].value, "alsa_output.bar.monitor");
        assert_eq!(devices[1].label, "alsa_output.bar.monitor");
        assert!(parse_pactl_devices("not json").is_empty());
    }

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

    #[test]
    fn display_service_results_never_claim_to_capture() {
        let cases = [
            (Ok(1), qol_headless::DoctorStatus::Ok),
            (
                Err("connection refused".to_string()),
                qol_headless::DoctorStatus::Fail,
            ),
        ];

        for (observation, status) in cases {
            let result = external_services_result(Some(":99".to_string()), observation);
            let details = result.details.unwrap();

            assert_eq!(result.status, status);
            assert_eq!(details["capture_attempted"], false);
        }
    }
}
