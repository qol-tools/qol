use qol_runtime::protocol::NotificationLevel;

pub fn show_already_running() {
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            "display notification \"Another instance is already running\" with title \"QoL Tray\"",
        ])
        .status();
}

pub fn show_first_run() {
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            "display notification \"QoL Tray is running. Click the menu bar icon to get started.\" with title \"QoL Tray\"",
        ])
        .status();
}

pub fn show_plugin_notification(
    title: &str,
    body: &str,
    _level: NotificationLevel,
    _action: Option<(&str, &str)>,
) {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape_applescript(body),
        escape_applescript(title)
    );
    std::thread::spawn(move || {
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .status();
    });
}

fn escape_applescript(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}
