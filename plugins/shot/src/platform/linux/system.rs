use crate::platform::AudioDevice;
use anyhow::{Context, Result};
use qol_audio::devices::{self, Device, Direction};
use qol_config::contract::{audio_device_picture, AudioDirection};
use qol_headless::DoctorCheckResult;
use qol_runtime::protocol::NotificationLevel;
use std::env;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use x11rb::connection::Connection;

pub fn show_notification(title: &str, message: &str, _timeout_ms: u32) {
    let client = qol_runtime::PlatformStateClient::from_env();
    if client.send_notification(title, message, NotificationLevel::Info) {
        return;
    }
    qol_plugin_daemon::notification::send_notification(title, message);
}

/// Saved-file notification with a clickable "Open Folder" action. Pushes
/// through the runtime channel with the action (payload = saved file path) so
/// the tray owns the click-through; falls back to the shared gate-aware
/// notification helper when the tray is unreachable (standalone run).
pub fn show_saved_notification(
    title: &str,
    message: &str,
    _timeout_ms: u32,
    target: crate::capture::completion::RevealTarget,
) {
    let client = qol_runtime::PlatformStateClient::from_env();
    let payload = target.path().to_string_lossy().into_owned();
    if client.send_notification_with_layout(
        title,
        message,
        NotificationLevel::Info,
        Some(("Open Folder", &payload)),
        Some(&payload),
        Some(crate::capture::completion::corner_toast_layout()),
    ) {
        return;
    }
    qol_plugin_daemon::notification::send_notification(title, message);
}

pub fn open_url(url: &str) -> Result<()> {
    qol_apps::desktop_integration::open_with_default_app(url).context("failed to open URL")
}

pub fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "platform_supported",
        "Linux capture uses Cinnamon's synchronized recorder when available and ffmpeg/x11grab elsewhere.",
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
    without_monitors(audio_devices(AudioDirection::Input))
}

pub fn list_audio_sinks() -> Vec<AudioDevice> {
    audio_devices(AudioDirection::Output)
}

fn audio_devices(direction: AudioDirection) -> Vec<AudioDevice> {
    match devices::list(native_direction(direction)) {
        Ok(devices) => devices
            .iter()
            .map(|device| map_audio_device(device, direction))
            .collect(),
        Err(error) => {
            eprintln!("[qol-shot] listing audio devices failed ({direction:?}): {error}");
            Vec::new()
        }
    }
}

fn native_direction(direction: AudioDirection) -> Direction {
    match direction {
        AudioDirection::Input => Direction::Input,
        AudioDirection::Output => Direction::Output,
    }
}

fn map_audio_device(device: &Device, direction: AudioDirection) -> AudioDevice {
    let entry = serde_json::json!({
        "name": &device.name,
        "properties": &device.properties,
        "active_port": &device.active_port,
    });
    AudioDevice {
        value: device.name.clone(),
        label: device.description.clone(),
        picture: audio_device_picture(&entry, direction).map(str::to_owned),
    }
}

fn without_monitors(devices: Vec<AudioDevice>) -> Vec<AudioDevice> {
    devices
        .into_iter()
        .filter(|device| !device.value.ends_with(".monitor"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_audio::devices::State;

    fn device(
        name: &str,
        description: &str,
        properties: &[(&str, &str)],
        active_port: Option<&str>,
    ) -> Device {
        Device {
            index: 1,
            name: name.to_owned(),
            description: description.to_owned(),
            properties: properties
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            active_port: active_port.map(str::to_owned),
            monitor_of_sink: None,
            state: State::Unknown,
        }
    }

    #[test]
    fn typed_devices_map_names_descriptions_and_pictures() {
        let devices = [
            device(
                "alsa_input.foo",
                "Built-in Microphone",
                &[("device.bus", "usb")],
                None,
            ),
            device("alsa_output.bar.monitor", "Monitor of bar", &[], None),
        ];
        let mapped = devices
            .iter()
            .map(|device| map_audio_device(device, AudioDirection::Input))
            .collect::<Vec<_>>();

        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0].value, "alsa_input.foo");
        assert_eq!(mapped[0].label, "Built-in Microphone");
        assert_eq!(mapped[0].picture.as_deref(), Some("usb-mic"));
        assert_eq!(mapped[1].value, "alsa_output.bar.monitor");
        assert_eq!(mapped[1].label, "Monitor of bar");
        assert_eq!(mapped[1].picture, None);
    }

    #[test]
    fn monitor_sources_are_excluded_from_source_listing() {
        let devices = [
            device("alsa_input.foo", "Built-in Microphone", &[], None),
            device("alsa_output.bar.monitor", "Monitor of bar", &[], None),
        ];
        let mapped = without_monitors(
            devices
                .iter()
                .map(|device| map_audio_device(device, AudioDirection::Input))
                .collect(),
        );

        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].value, "alsa_input.foo");
    }

    #[test]
    fn output_devices_map_their_audio_picture() {
        let devices = [
            device(
                "alsa_output.usb",
                "USB Headset",
                &[("device.form_factor", "headset")],
                None,
            ),
            device("alsa_output.pci", "Speakers", &[], None),
            device(
                "alsa_output.digital",
                "Digital Output",
                &[],
                Some("hdmi-stereo"),
            ),
        ];
        let mapped = devices
            .iter()
            .map(|device| map_audio_device(device, AudioDirection::Output))
            .collect::<Vec<_>>();

        assert_eq!(mapped[0].picture.as_deref(), Some("headphones"));
        assert_eq!(mapped[1].picture, None);
        assert_eq!(mapped[2].picture.as_deref(), Some("hdmi"));
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
