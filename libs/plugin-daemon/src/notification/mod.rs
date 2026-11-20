pub mod gate;
mod platform;

pub use platform::send_notification as send_native_notification;
pub use platform::{
    acquire_inhibit, install_toast_identity, os_banners_showing, os_do_not_disturb,
    release_toast_identity, set_os_banners, uninstall_toast_identity, NotificationInhibit,
};

pub use qol_runtime::protocol::{NotificationLayout, NotificationLevel, NotificationMeta};
pub use qol_runtime::Notification;

use qol_runtime::protocol::DaemonRequest;
use qol_runtime::PlatformStateClient;

/// Delivers a notification through [`send`]: the tray host over the runtime
/// state socket when one is reachable, else the platform notifier
/// (`notify-send` on Linux, `osascript` on macOS, a toast on Windows), else
/// the process log. Nothing is shown while notifications are switched off.
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
    send(&Notification {
        layout,
        ..Notification::new(title, message)
    });
}

pub fn send_banner(title: &str, message: &str, value: Option<&str>) {
    send_notification_with_layout(title, message, Some(NotificationLayout::banner(value)));
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
    send(&Notification {
        layout,
        activate: Some(activate),
        ..Notification::new(title, message)
    });
}

pub fn send(notification: &Notification) -> bool {
    deliver_in_order(
        gate::enabled_now(),
        notification,
        |notification| PlatformStateClient::from_env().push(notification),
        platform::send,
    )
}

fn deliver_in_order(
    enabled: bool,
    notification: &Notification,
    push: impl FnOnce(&Notification) -> bool,
    os: impl FnOnce(&Notification) -> bool,
) -> bool {
    if !enabled {
        log::info!(
            "notifications are switched off: {}: {}",
            notification.title,
            notification.body
        );
        return false;
    }
    if push(notification) || os(notification) {
        return true;
    }

    log::info!("{}: {}", notification.title, notification.body);
    false
}

#[cfg(test)]
mod tests;
