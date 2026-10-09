use qol_audio::devices;
use qol_headless::DoctorCheckResult;

use super::display::native_displays;
use super::ffmpeg;
use crate::platform::AudioDevice;

pub fn process_alive(pid: u32) -> bool {
    qol_process::is_pid_alive(pid)
}

pub fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "platform_supported",
        "Windows screenshots use GDI screen capture; recordings use ffmpeg gdigrab with WASAPI microphone and system audio.",
    )
}

pub fn required_binaries_check() -> DoctorCheckResult {
    required_binaries_result(ffmpeg::resolve_ffmpeg())
}

fn required_binaries_result(ffmpeg: Option<std::path::PathBuf>) -> DoctorCheckResult {
    let details = serde_json::json!({
        "platform": "windows",
        "ffmpeg": ffmpeg.as_ref().map(|path| path.display().to_string()),
    });
    match ffmpeg {
        Some(path) => DoctorCheckResult::ok(
            "required_binaries",
            format!("Screen recording uses ffmpeg at {}.", path.display()),
        )
        .with_details(details),
        None => DoctorCheckResult::fail(
            "required_binaries",
            "Missing required binaries: ffmpeg. Screenshots work; screen recording needs ffmpeg.",
        )
        .with_fix(ffmpeg::INSTALL_HINT)
        .with_details(details),
    }
}

pub fn external_services_check() -> DoctorCheckResult {
    let displays = native_displays().len();
    let details = serde_json::json!({
        "platform": "windows",
        "service": "gdi",
        "display_count": displays,
        "capture_attempted": false,
    });
    if displays == 0 {
        return DoctorCheckResult::fail(
            "external_services",
            "Windows reported no display monitors to capture.",
        )
        .with_details(details)
        .with_fix("Run QoL Shot from an interactive desktop session.");
    }
    DoctorCheckResult::ok(
        "external_services",
        format!("Windows reports {displays} display monitor(s) for GDI capture."),
    )
    .with_details(details)
}

pub fn list_audio_sources() -> Vec<AudioDevice> {
    audio_devices("input", devices::list_inputs())
}

pub fn list_audio_sinks() -> Vec<AudioDevice> {
    audio_devices("output", devices::list_outputs())
}

fn audio_devices(
    direction: &str,
    listed: Result<Vec<devices::AudioDevice>, qol_audio::AudioError>,
) -> Vec<AudioDevice> {
    match listed {
        Ok(listed) => listed.into_iter().map(map_audio_device).collect(),
        Err(error) => {
            log::warn!("listing audio devices failed ({direction}): {error}");
            Vec::new()
        }
    }
}

fn map_audio_device(device: devices::AudioDevice) -> AudioDevice {
    AudioDevice {
        value: device.value,
        label: device.label,
        picture: Some(device.picture),
    }
}

#[cfg(test)]
mod tests {
    use super::{audio_devices, required_binaries_result};
    use qol_audio::devices::{AudioDevice, Identity, Kind};
    use qol_headless::DoctorStatus;
    use std::path::PathBuf;

    #[test]
    fn missing_ffmpeg_fails_with_the_winget_hint() {
        let cases = [
            (
                Some(PathBuf::from(r"C:fmpeginfmpeg.exe")),
                DoctorStatus::Ok,
                None,
            ),
            (None, DoctorStatus::Fail, Some("winget install Gyan.FFmpeg")),
        ];
        for (ffmpeg, status, fix) in cases {
            let result = required_binaries_result(ffmpeg.clone());
            assert_eq!(result.status, status, "{ffmpeg:?}");
            match fix {
                Some(fix) => assert!(result.fix.as_deref().unwrap_or("").contains(fix)),
                None => assert!(result.fix.is_none()),
            }
        }
    }

    #[test]
    fn listed_devices_keep_their_endpoint_id_label_and_picture() {
        let listed = vec![AudioDevice {
            identity: Identity::from_raw("{0.0.1.00000000}.{mic}"),
            value: "{0.0.1.00000000}.{mic}".to_string(),
            label: "USB Microphone".to_string(),
            picture: "usb-mic".to_string(),
            kind: Kind::Usb,
        }];
        let mapped = audio_devices("input", Ok(listed));
        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].value, "{0.0.1.00000000}.{mic}");
        assert_eq!(mapped[0].label, "USB Microphone");
        assert_eq!(mapped[0].picture.as_deref(), Some("usb-mic"));
        assert!(audio_devices("output", Err(qol_audio::AudioError::Unsupported)).is_empty());
    }
}
