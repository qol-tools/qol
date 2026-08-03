use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::MonitorBounds;

// ── Daemon action protocol (qol-tray ↔ plugin daemon) ──────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonRequest {
    pub action: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub input: serde_json::Value,
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
    GetPluginConfig {
        plugin_id: String,
    },
    SetPluginConfig {
        plugin_id: String,
        config: serde_json::Value,
    },
    PushNotification {
        plugin_id: String,
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        level: NotificationLevel,
        /// Optional action rendered as a clickable button by the host's
        /// notification surface (notify-send `-A` on Linux). Both fields must
        /// be present for the host to render the action; V1 interprets the
        /// payload as a path it opens with the default app when invoked.
        #[serde(default)]
        action_label: Option<String>,
        #[serde(default)]
        action_payload: Option<String>,
    },
    PushStatus {
        plugin_id: String,
        status: serde_json::Value,
    },
}

/// Severity of a plugin-pushed notification, mapped by the host to its own
/// notification surface (e.g. notify-send urgency on Linux).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationLevel {
    #[default]
    Info,
    Warn,
    Error,
}

/// Ack for the plugin-to-host push requests (`PushNotification`, `PushStatus`).
/// `Handled` means the host accepted the push; `Error` means it rejected it
/// (for example an unknown plugin id).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PushAck {
    Handled,
    Error {
        #[serde(default)]
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PluginConfigResponse {
    Ok {
        #[serde(default)]
        config: serde_json::Value,
    },
    Error {
        message: String,
    },
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
    fn plugin_config_request_round_trips() {
        let request = RuntimeRequest::SetPluginConfig {
            plugin_id: "plugin-foo".to_string(),
            config: serde_json::json!({ "enabled": true }),
        };
        let wire = serde_json::to_string(&request).expect("serialize");
        let RuntimeRequest::SetPluginConfig { plugin_id, config } =
            serde_json::from_str(&wire).expect("deserialize")
        else {
            panic!("variant mismatch");
        };
        assert_eq!(plugin_id, "plugin-foo");
        assert_eq!(config, serde_json::json!({ "enabled": true }));
    }

    #[test]
    fn plugin_config_response_round_trips() {
        let cases = [
            (
                PluginConfigResponse::Ok {
                    config: serde_json::json!({ "n": 1 }),
                },
                "ok",
            ),
            (
                PluginConfigResponse::Error {
                    message: "boom".to_string(),
                },
                "error",
            ),
        ];
        for (response, status) in cases {
            let wire = serde_json::to_string(&response).expect("serialize");
            assert!(wire.contains(status), "status {status} in {wire}");
            let parsed: PluginConfigResponse = serde_json::from_str(&wire).expect("deserialize");
            assert_eq!(
                serde_json::to_string(&parsed).unwrap(),
                wire,
                "round trip for {status}"
            );
        }
    }

    #[test]
    fn event_kind_serializes_in_snake_case() {
        let kind = RuntimeEventKind::LauncherAppsSynced;
        assert_eq!(
            serde_json::to_string(&kind).unwrap(),
            "\"launcher_apps_synced\""
        );
    }

    #[test]
    fn push_notification_round_trips() {
        let request = RuntimeRequest::PushNotification {
            plugin_id: "plugin-foo".to_string(),
            title: "Backup complete".to_string(),
            body: "3 files synced".to_string(),
            level: NotificationLevel::Info,
            action_label: None,
            action_payload: None,
        };
        let wire = serde_json::to_string(&request).expect("serialize");
        assert!(wire.contains("\"cmd\":\"push_notification\""));
        assert!(wire.contains("\"level\":\"info\""));

        let RuntimeRequest::PushNotification {
            plugin_id,
            title,
            body,
            level,
            action_label,
            action_payload,
        } = serde_json::from_str(&wire).expect("deserialize")
        else {
            panic!("variant mismatch");
        };
        assert_eq!(plugin_id, "plugin-foo");
        assert_eq!(title, "Backup complete");
        assert_eq!(body, "3 files synced");
        assert_eq!(level, NotificationLevel::Info);
        assert_eq!(action_label, None);
        assert_eq!(action_payload, None);
    }

    #[test]
    fn push_notification_with_action_round_trips() {
        let request = RuntimeRequest::PushNotification {
            plugin_id: "plugin-shot".to_string(),
            title: "Recording saved".to_string(),
            body: "video file".to_string(),
            level: NotificationLevel::Info,
            action_label: Some("Open Folder".to_string()),
            action_payload: Some("/home/u/Videos/qol-shot.mp4".to_string()),
        };
        let wire = serde_json::to_string(&request).expect("serialize");
        assert!(wire.contains("\"action_label\":\"Open Folder\""));
        assert!(wire.contains("\"action_payload\":\"/home/u/Videos/qol-shot.mp4\""));

        let RuntimeRequest::PushNotification {
            action_label,
            action_payload,
            ..
        } = serde_json::from_str(&wire).expect("deserialize")
        else {
            panic!("variant mismatch");
        };
        assert_eq!(action_label.as_deref(), Some("Open Folder"));
        assert_eq!(
            action_payload.as_deref(),
            Some("/home/u/Videos/qol-shot.mp4")
        );
    }

    #[test]
    fn push_notification_defaults_body_and_level_when_missing() {
        let wire = r#"{"cmd":"push_notification","plugin_id":"plugin-foo","title":"Hi"}"#;
        let RuntimeRequest::PushNotification {
            body,
            level,
            action_label,
            action_payload,
            ..
        } = serde_json::from_str(wire).expect("deserialize")
        else {
            panic!("variant mismatch");
        };
        assert_eq!(body, "");
        assert_eq!(level, NotificationLevel::Info);
        assert_eq!(action_label, None);
        assert_eq!(action_payload, None);
    }

    #[test]
    fn push_status_round_trips() {
        let request = RuntimeRequest::PushStatus {
            plugin_id: "plugin-foo".to_string(),
            status: serde_json::json!({ "state": "recording", "elapsed_s": 12 }),
        };
        let wire = serde_json::to_string(&request).expect("serialize");
        assert!(wire.contains("\"cmd\":\"push_status\""));

        let RuntimeRequest::PushStatus { plugin_id, status } =
            serde_json::from_str(&wire).expect("deserialize")
        else {
            panic!("variant mismatch");
        };
        assert_eq!(plugin_id, "plugin-foo");
        assert_eq!(
            status,
            serde_json::json!({ "state": "recording", "elapsed_s": 12 })
        );
    }

    #[test]
    fn push_ack_round_trips() {
        let cases = [
            (PushAck::Handled, "handled"),
            (
                PushAck::Error {
                    message: "unknown plugin".to_string(),
                },
                "error",
            ),
        ];
        for (ack, status) in cases {
            let wire = serde_json::to_string(&ack).expect("serialize");
            assert!(wire.contains(status), "status {status} in {wire}");
            let parsed: PushAck = serde_json::from_str(&wire).expect("deserialize");
            assert_eq!(
                serde_json::to_string(&parsed).unwrap(),
                wire,
                "round trip for {status}"
            );
        }
    }

    #[test]
    fn notification_level_serializes_in_snake_case() {
        for (level, expected) in [
            (NotificationLevel::Info, "info"),
            (NotificationLevel::Warn, "warn"),
            (NotificationLevel::Error, "error"),
        ] {
            assert_eq!(
                serde_json::to_string(&level).unwrap(),
                format!("\"{expected}\"")
            );
        }
    }
}
