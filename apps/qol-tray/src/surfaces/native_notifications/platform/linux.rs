use qol_conventions::DEFAULT_PORT;
use qol_runtime::protocol::NotificationLevel;

pub fn show_already_running() {
    let _ = std::process::Command::new("notify-send")
        .args(["QoL Tray", "Another instance is already running"])
        .status();
}

pub fn show_first_run() {
    let message = format!(
        "QoL Tray is running. Click the tray icon or visit http://localhost:{DEFAULT_PORT} to get started."
    );
    let _ = std::process::Command::new("notify-send")
        .args(["--icon=qol-tray", "QoL Tray", &message])
        .status();
}

pub fn show_plugin_notification(
    title: &str,
    body: &str,
    level: NotificationLevel,
    action: Option<(&str, &str)>,
) {
    let urgency = match level {
        NotificationLevel::Info => "low",
        NotificationLevel::Warn => "normal",
        NotificationLevel::Error => "critical",
    };
    notify_send_detached(
        title.to_string(),
        body.to_string(),
        urgency.to_string(),
        action.map(|(label, payload)| (label.to_string(), payload.to_string())),
    );
}

fn notify_send_detached(
    title: String,
    body: String,
    urgency: String,
    action: Option<(String, String)>,
) {
    std::thread::spawn(move || {
        let mut command = std::process::Command::new("notify-send");
        command.args(["-a", "QoL Tray", "-u", urgency.as_str()]);
        let Some((label, payload)) = action else {
            let _ = command.arg(title).arg(body).status();
            return;
        };
        command
            .args(["-A", &format!("open={label}")])
            .arg(title)
            .arg(body)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let Ok(output) = command.output() else {
            return;
        };
        if !output.status.success() {
            return;
        }
        if String::from_utf8_lossy(&output.stdout).trim() != "open" {
            return;
        }
        if let Err(error) = qol_apps::desktop_integration::open_with_default_app(&payload) {
            log::warn!("[notifications] action {label:?} payload open failed: {error:#}");
        }
    });
}
