use qol_headless::DoctorCheckResult;

use super::display::native_displays;

pub fn process_alive(pid: u32) -> bool {
    qol_process::is_pid_alive(pid)
}

pub fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "platform_supported",
        "Windows screenshots use GDI screen capture; screen recording is not supported on Windows yet.",
    )
}

pub fn required_binaries_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "required_binaries",
        "Windows screenshots need no external binaries.",
    )
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

pub fn list_audio_sources() -> Vec<crate::platform::AudioDevice> {
    Vec::new()
}

pub fn list_audio_sinks() -> Vec<crate::platform::AudioDevice> {
    Vec::new()
}
