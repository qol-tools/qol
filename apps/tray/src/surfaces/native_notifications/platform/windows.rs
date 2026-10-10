use qol_conventions::{DEFAULT_PORT, TRAY_DISPLAY_NAME};
use qol_plugin_daemon::notification::platform::send_notification;
use qol_runtime::protocol::NotificationLevel;

pub(crate) fn show_already_running() {
    send_notification(TRAY_DISPLAY_NAME, "Another instance is already running");
}

pub(crate) fn show_first_run() {
    let message = format!(
        "QoL Tray is running. Click the tray icon or visit http://localhost:{DEFAULT_PORT} to get started."
    );
    send_notification(TRAY_DISPLAY_NAME, &message);
}

pub(crate) fn show_plugin_notification(
    title: &str,
    body: &str,
    _level: NotificationLevel,
    _action: Option<(&str, &str)>,
) {
    let title = title.to_string();
    let body = body.to_string();
    std::thread::spawn(move || {
        send_notification(&title, &body);
    });
}
