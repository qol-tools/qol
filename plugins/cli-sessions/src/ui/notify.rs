use crate::host::project_of;
use crate::session::status::Status;
use crate::session::tool::{is_generic, Tool};
use qol_runtime::protocol::{DaemonRequest, NotificationLayout};
use qol_sessions_engine::fork_watch::{ForkWatch, WatchState};
use qol_terminal_sessions::park::{ParkRecord, ParkState};
use qol_terminal_sessions::SessionId;

pub const FOCUS_ACTION: &str = "focus";
pub const REOPEN_ACTION: &str = "reopen";
pub const SWITCHBOARD_ACTION: &str = "switchboard";
const FORK_TITLE: &str = "Switchboard fork";
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

    pub fn for_park(record: &ParkRecord) -> Self {
        if record.state == ParkState::Failed {
            Self::failed_park(record)
        } else {
            Self::finished_park(record)
        }
    }

    /// The final message of a woken park whose tab closed; a click reopens the conversation.
    pub fn finished_park(record: &ParkRecord) -> Self {
        Self::park_notice(record, record.report.as_deref().unwrap_or_default())
    }

    pub fn failed_park(record: &ParkRecord) -> Self {
        let reason = record.detail.as_deref().unwrap_or("no reason was recorded");
        Self::park_notice(record, &format!("Could not resume: {reason}"))
    }

    fn park_notice(record: &ParkRecord, text: &str) -> Self {
        let text = text.trim();
        let mut body: String = text.chars().take(REPORT_MAX_CHARS).collect();
        if body.len() < text.len() {
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

    pub fn for_fork_watch(watch: &ForkWatch) -> Self {
        let name = watch
            .name
            .as_deref()
            .map(|name| name.split(':').next().unwrap_or(name).trim())
            .filter(|name| !name.is_empty())
            .unwrap_or(FORK_TITLE);
        let what = watch
            .pull_request
            .as_deref()
            .and_then(|pull_request| pull_request.rsplit('/').next())
            .map(|number| format!("The watcher of #{number}"))
            .unwrap_or_else(|| "A watched command".to_owned());
        let body = if watch.state == WatchState::Failed {
            format!("{what} stopped: CLI Sessions lost its runner. Open the switchboard to tell the fork.")
        } else {
            format!("{what} finished. Open the switchboard to give the fork its result.")
        };
        Self {
            title: name.to_owned(),
            body,
            activate: switchboard_request(),
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

pub fn switchboard_request() -> DaemonRequest {
    DaemonRequest {
        action: SWITCHBOARD_ACTION.to_string(),
        input: serde_json::Value::Null,
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
