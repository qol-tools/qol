pub mod native_notifications;

use qol_runtime::protocol::{NotificationLayout, NotificationLevel};

pub fn show_plugin_notification(
    title: &str,
    body: &str,
    level: NotificationLevel,
    action: Option<(&str, &str)>,
    artifact: Option<&str>,
    layout: Option<NotificationLayout>,
) {
    let system_notifications = crate::features::notifications::use_system_notifications();
    let toast_shown = !system_notifications
        && crate::settings_surface::show_toast(
            title,
            body,
            notification_level_name(level),
            action,
            artifact,
            layout,
        )
        .unwrap_or(false);
    if !toast_shown {
        native_notifications::show_plugin_notification(title, body, level, action);
    }
}

fn notification_level_name(level: NotificationLevel) -> &'static str {
    match level {
        NotificationLevel::Info => "info",
        NotificationLevel::Warn => "warn",
        NotificationLevel::Error => "error",
    }
}
