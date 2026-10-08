use crate::host::project_of;
use crate::session::status::Status;
use crate::session::tool::{is_generic, Tool};
use qol_runtime::protocol::{DaemonRequest, NotificationLayout};
use qol_terminal_sessions::park::ParkRecord;
use qol_terminal_sessions::SessionId;

pub const FOCUS_ACTION: &str = "focus";
pub const REOPEN_ACTION: &str = "reopen";
const REPORT_MAX_CHARS: usize = 400;

pub struct Notice {
    pub title: String,
    pub body: String,
    activate: DaemonRequest,
}

impl Notice {
    pub fn new(session: SessionId, tool: &Tool, label: String, summary: &str) -> Self {
        let prefix = if is_generic(tool) {
            String::new()
        } else {
            format!("{} \u{00B7} ", tool.label)
        };
        Self {
            activate: focus_request(&session),
            title: label,
            body: format!("{prefix}{summary}"),
        }
    }

    /// The final message of a woken park whose tab closed; a click reopens the conversation.
    pub fn finished_park(record: &ParkRecord) -> Self {
        let report = record.report.as_deref().unwrap_or_default().trim();
        let mut body: String = report.chars().take(REPORT_MAX_CHARS).collect();
        if body.len() < report.len() {
            body.push('\u{2026}');
        }
        Self {
            title: record
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| project_of(&record.cwd)),
            body,
            activate: reopen_request(&record.id),
        }
    }

    /// The action a click on the toast sends back.
    pub fn activate(&self) -> DaemonRequest {
        self.activate.clone()
    }
}

pub fn focus_request(session: &SessionId) -> DaemonRequest {
    DaemonRequest {
        action: FOCUS_ACTION.to_string(),
        input: serde_json::json!({ "session": session }),
    }
}

pub fn reopen_request(park: &str) -> DaemonRequest {
    DaemonRequest {
        action: REOPEN_ACTION.to_string(),
        input: serde_json::json!({ "park": park }),
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
