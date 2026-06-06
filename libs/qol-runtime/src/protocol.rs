use std::path::PathBuf;

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
    SetFocus {
        monitor_idx: usize,
    },
    Subscribe {
        #[serde(default)]
        plugin_id: String,
        events: Vec<RuntimeEventKind>,
    },
    Lifeline {
        plugin_id: String,
    },
    ArmedLifelines,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEventKind {
    ActiveMonitorChanged,
    CursorMoved,
    FocusChanged,
    LauncherAppsSynced,
    MonitorsChanged,
    WindowListChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RuntimeEvent {
    ActiveMonitorChanged {
        monitor_idx: Option<usize>,
        monitor: Option<MonitorBounds>,
    },
    CursorMoved {
        x: f32,
        y: f32,
    },
    FocusChanged {
        monitor_idx: Option<usize>,
        monitor: Option<MonitorBounds>,
    },
    LauncherAppsSynced {
        #[serde(default)]
        dir: PathBuf,
    },
    MonitorsChanged {
        monitors: Vec<MonitorBounds>,
    },
    WindowListChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SubscribeAck {
    Subscribed,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmedLifelinesResponse {
    pub plugin_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(event: &RuntimeEvent) -> RuntimeEvent {
        let wire = serde_json::to_string(event).expect("serialize");
        serde_json::from_str(&wire).expect("deserialize")
    }

    #[test]
    fn launcher_apps_synced_round_trips() {
        let event = RuntimeEvent::LauncherAppsSynced {
            dir: PathBuf::from("/home/u/Applications/QoL"),
        };
        let RuntimeEvent::LauncherAppsSynced { dir } = roundtrip(&event) else {
            panic!("variant mismatch");
        };
        assert_eq!(dir, PathBuf::from("/home/u/Applications/QoL"));
    }

    #[test]
    fn launcher_apps_synced_defaults_dir_when_missing() {
        let wire = r#"{"event":"launcher_apps_synced"}"#;
        let event: RuntimeEvent = serde_json::from_str(wire).expect("deserialize");
        let RuntimeEvent::LauncherAppsSynced { dir } = event else {
            panic!("variant mismatch");
        };
        assert_eq!(dir, PathBuf::new());
    }

    #[test]
    fn event_kind_serializes_in_snake_case() {
        let kind = RuntimeEventKind::LauncherAppsSynced;
        assert_eq!(
            serde_json::to_string(&kind).unwrap(),
            "\"launcher_apps_synced\""
        );
    }
}
