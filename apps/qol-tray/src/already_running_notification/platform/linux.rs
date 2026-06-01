pub(crate) fn show() {
    let _ = std::process::Command::new("notify-send")
        .args(["QoL Tray", "Another instance is already running"])
        .status();
}
