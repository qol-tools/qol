use qol_conventions::DEFAULT_PORT;

pub(crate) fn show_already_running() {
    let _ = std::process::Command::new("notify-send")
        .args(["QoL Tray", "Another instance is already running"])
        .status();
}

pub(crate) fn show_first_run() {
    let message = format!(
        "QoL Tray is running. Click the tray icon or visit http://localhost:{DEFAULT_PORT} to get started."
    );
    let _ = std::process::Command::new("notify-send")
        .args(["--icon=qol-tray", "QoL Tray", &message])
        .status();
}
