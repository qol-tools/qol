pub mod gate;
pub mod platform;

use qol_runtime::protocol::{DaemonRequest, NotificationLayout, NotificationLevel};
use qol_runtime::PlatformStateClient;

/// Delivers a notification, push-first: sends it to the tray host over the
/// runtime state socket when one is reachable, and falls back to the
/// platform notifier (`notify-send` on Linux, `osascript` on macOS, a toast on
/// Windows) and finally to the process log when the push is unavailable or
/// rejected.
/// Callers pass no urgency, so the push always carries
/// [`NotificationLevel::Info`].
pub fn send_notification(title: &str, message: &str) {
    send_notification_with_layout(title, message, None);
}

pub fn send_notification_with_layout(
    title: &str,
    message: &str,
    layout: Option<NotificationLayout>,
) {
    deliver(title, message, |client| {
        client.send_notification_with_layout(
            title,
            message,
            NotificationLevel::Info,
            None,
            None,
            layout,
        )
    });
}

/// Like [`send_notification_with_layout`], and a click on the tray's toast
/// sends `activate` back to this plugin as a regular plugin action. The
/// platform fallback cannot carry the click, so it shows a plain notification.
pub fn send_activatable_notification(
    title: &str,
    message: &str,
    layout: Option<NotificationLayout>,
    activate: DaemonRequest,
) {
    deliver(title, message, |client| {
        client.send_activatable_notification(
            title,
            message,
            NotificationLevel::Info,
            layout,
            activate,
        )
    });
}

fn deliver(title: &str, message: &str, push: impl FnOnce(&PlatformStateClient) -> bool) {
    if push(&PlatformStateClient::from_env()) {
        return;
    }
    if gate::native_allowed_now() && platform::send_notification(title, message) {
        return;
    }

    log::info!("{title}: {message}");
}
