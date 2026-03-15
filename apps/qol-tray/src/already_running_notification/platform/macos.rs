pub(crate) fn show() {
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            "display notification \"Another instance is already running\" with title \"QoL Tray\"",
        ])
        .status();
}
