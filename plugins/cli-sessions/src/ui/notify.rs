use crate::session::status::Status;
use crate::session::tool::{is_generic, Tool};
use qol_runtime::protocol::NotificationLayout;

pub struct Notice {
    pub title: String,
    pub body: String,
}

impl Notice {
    pub fn new(tool: &Tool, label: String, summary: &str) -> Self {
        let prefix = if is_generic(tool) {
            String::new()
        } else {
            format!("{} \u{00B7} ", tool.label)
        };
        Self {
            title: label,
            body: format!("{prefix}{summary}"),
        }
    }
}

pub fn announces_attention(prev: Status, new: Status) -> bool {
    new != prev && new.is_attention()
}

pub fn send(notice: &Notice) {
    qol_plugin_daemon::notification::send_notification_with_layout(
        &notice.title,
        &notice.body,
        Some(NotificationLayout {
            anchor: Some("bottom-right".to_string()),
            width: Some(380.0),
            height: Some(76.0),
            style: Some("compact".to_string()),
        }),
    );
}
