use crate::session::status::Status;
use crate::session::tool::{is_generic, Tool};
use qol_runtime::protocol::{DaemonRequest, NotificationLayout};
use qol_terminal_sessions::SessionId;

pub const FOCUS_ACTION: &str = "focus";

pub struct Notice {
    pub session: SessionId,
    pub title: String,
    pub body: String,
}

impl Notice {
    pub fn new(session: SessionId, tool: &Tool, label: String, summary: &str) -> Self {
        let prefix = if is_generic(tool) {
            String::new()
        } else {
            format!("{} \u{00B7} ", tool.label)
        };
        Self {
            session,
            title: label,
            body: format!("{prefix}{summary}"),
        }
    }

    /// The action a click on the toast sends back: focus this session.
    pub fn activate(&self) -> DaemonRequest {
        focus_request(&self.session)
    }
}

pub fn focus_request(session: &SessionId) -> DaemonRequest {
    DaemonRequest {
        action: FOCUS_ACTION.to_string(),
        input: serde_json::json!({ "session": session }),
    }
}

pub fn announces_attention(prev: Status, new: Status) -> bool {
    new != prev && new.is_attention()
}

pub fn send(notice: &Notice) {
    qol_plugin_daemon::notification::send_activatable_notification(
        &notice.title,
        &notice.body,
        Some(NotificationLayout {
            style: Some("compact".to_string()),
            ..Default::default()
        }),
        notice.activate(),
    );
}
