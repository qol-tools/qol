use serde::{Deserialize, Serialize};

use crate::MonitorBounds;

// ── Daemon action protocol (qol-tray ↔ plugin daemon) ──────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonRequest {
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DaemonResponse {
    Handled {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
    Fallback,
    Error {
        #[serde(default)]
        message: String,
    },
}

// ── Runtime state protocol (plugin ↔ qol-tray runtime server) ──────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum RuntimeRequest {
    GetState,
    SetFocus { monitor_idx: usize },
    Subscribe { events: Vec<RuntimeEventKind> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEventKind {
    ActiveMonitorChanged,
    FocusChanged,
    MonitorsChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RuntimeEvent {
    ActiveMonitorChanged {
        monitor_idx: Option<usize>,
        monitor: Option<MonitorBounds>,
    },
    FocusChanged {
        monitor_idx: Option<usize>,
        monitor: Option<MonitorBounds>,
    },
    MonitorsChanged {
        monitors: Vec<MonitorBounds>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SubscribeAck {
    Subscribed,
    Error { message: String },
}
