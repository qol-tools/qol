pub mod native_notifications;

use qol_plugin_daemon::notification::gate::NativeHandler;
use qol_runtime::protocol::{DaemonRequest, NotificationLayout, NotificationLevel};

#[allow(clippy::too_many_arguments)]
pub fn show_plugin_notification(
    plugin_id: Option<&str>,
    title: &str,
    body: &str,
    level: NotificationLevel,
    action: Option<(&str, &str)>,
    artifact: Option<&str>,
    layout: Option<NotificationLayout>,
    activate: Option<&DaemonRequest>,
) {
    let handler = crate::features::notifications::native_handler();
    let toast_shown = handler != NativeHandler::Os && {
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
            plugin_id.zip(activate),
        )
        .unwrap_or(false)
    };
    if !toast_shown || handler == NativeHandler::Both {
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
