mod platform;

use qol_runtime::protocol::NotificationLevel;

/// Delivers a notification, push-first: sends it to the tray host over the
/// runtime state socket when one is reachable, and falls back to the
/// platform shell-out (`notify-send` on Linux, `osascript` on macOS) and
/// finally to stdout when the push is unavailable or rejected. Callers pass
/// no urgency, so the push always carries [`NotificationLevel::Info`].
pub fn send_notification(title: &str, message: &str) {
    let client = qol_runtime::PlatformStateClient::from_env();
    if client.send_notification(title, message, NotificationLevel::Info) {
        return;
    }
    if platform::send_notification(title, message) {
        return;
    }

    println!("{title}: {message}");
}
