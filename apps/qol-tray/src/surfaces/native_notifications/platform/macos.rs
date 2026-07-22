pub(crate) fn show_already_running() {
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            "display notification \"Another instance is already running\" with title \"QoL Tray\"",
        ])
        .status();
}

pub(crate) fn show_first_run() {
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            "display notification \"QoL Tray is running. Click the menu bar icon to get started.\" with title \"QoL Tray\"",
        ])
        .status();
}
