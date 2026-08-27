mod platform;

use qol_runtime::protocol::{NotificationLayout, NotificationLevel};

/// Delivers a notification, push-first: sends it to the tray host over the
/// runtime state socket when one is reachable, and falls back to the
/// platform shell-out (`notify-send` on Linux, `osascript` on macOS) and
/// finally to stdout when the push is unavailable or rejected. Callers pass
/// no urgency, so the push always carries [`NotificationLevel::Info`].
pub fn send_notification(title: &str, message: &str) {
    send_notification_with_layout(title, message, None);
}

pub fn send_notification_with_layout(
    title: &str,
    message: &str,
    layout: Option<NotificationLayout>,
) {
    let client = qol_runtime::PlatformStateClient::from_env();
    if client.send_notification_with_layout(
        title,
        message,
        NotificationLevel::Info,
        None,
        None,
        layout,
    ) {
        return;
    }
    if platform::send_notification(title, message) {
        return;
    }

    println!("{title}: {message}");
}
