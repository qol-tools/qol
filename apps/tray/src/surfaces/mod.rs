pub mod native_notifications;

use qol_runtime::protocol::{NotificationLayout, NotificationLevel};

pub fn show_plugin_notification(
    plugin_id: Option<&str>,
    title: &str,
    body: &str,
    level: NotificationLevel,
    action: Option<(&str, &str)>,
    artifact: Option<&str>,
    layout: Option<NotificationLayout>,
) {
    let toast_shown = !crate::features::notifications::use_system_notifications() && {
        let (name, mark) = plugin_id.map_or_else(
            || (qol_conventions::TRAY_DISPLAY_NAME.to_string(), None),
            crate::plugins::name_and_icon,
        );
        crate::settings_surface::show_toast(
            crate::settings_surface::ToastSource {
                group: plugin_id.unwrap_or(qol_conventions::TRAY_DISPLAY_NAME),
                name: &name,
                mark: mark.as_deref(),
            },
            title,
            body,
            notification_level_name(level),
            action,
            artifact,
            layout,
        )
        .unwrap_or(false)
    };
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
