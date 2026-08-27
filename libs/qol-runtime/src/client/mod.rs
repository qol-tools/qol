use crate::protocol::{
    ArmedLifelinesResponse, NotificationLayout, NotificationLevel, PluginConfigResponse, PushAck,
    RuntimeEvent, RuntimeEventKind, RuntimeRequest, SubscribeAck,
};
use crate::PlatformState;
use serde::Serialize;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Duration;

use qol_conventions::{ENV_PLUGIN_ID, ENV_STATE_SOCKET};

/// Connection abstraction shared with `broker::client` (the event bus
/// client connects to the broker socket the same way).
pub(crate) mod platform;

const TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub struct PlatformStateClient {
    socket_path: PathBuf,
}

pub struct Subscription {
    reader: BufReader<Box<dyn platform::Connection>>,
}

impl PlatformStateClient {
    pub fn from_env() -> Self {
        let path = std::env::var_os(ENV_STATE_SOCKET)
            .map(PathBuf::from)
            .or_else(|| {
                qol_config::runtime_dir().map(|path| {
                    path.join("sockets")
                        .join(qol_conventions::STATE_SOCKET_FILE)
                })
            })
            .unwrap_or_else(|| PathBuf::from(qol_conventions::STATE_SOCKET_PATH));
        Self { socket_path: path }
    }

    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    pub fn get_state(&self) -> Option<PlatformState> {
        let mut reader = self.send(&RuntimeRequest::GetState, Some(TIMEOUT))?;
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;

        serde_json::from_str(&line).ok()
    }

    fn send(
        &self,
        request: &impl Serialize,
        read_timeout: Option<Duration>,
    ) -> Option<BufReader<Box<dyn platform::Connection>>> {
        let mut stream = platform::connect(&self.socket_path).ok()?;
        stream.set_read_timeout(read_timeout).ok()?;
        stream.set_write_timeout(Some(TIMEOUT)).ok()?;

        let mut payload = serde_json::to_string(request).ok()?;
        payload.push('\n');
        stream.write_all(payload.as_bytes()).ok()?;

        Some(BufReader::new(stream))
    }

    pub fn set_focus(&self, monitor_idx: usize) {
        let Ok(mut stream) = platform::connect(&self.socket_path) else {
            return;
        };
        let _ = stream.set_write_timeout(Some(TIMEOUT));
        let request = RuntimeRequest::SetFocus { monitor_idx };
        let Ok(mut payload) = serde_json::to_string(&request) else {
            return;
        };
        payload.push('\n');
        let _ = stream.write_all(payload.as_bytes());
    }

    pub fn get_plugin_config(&self, plugin_id: &str) -> Option<serde_json::Value> {
        let request = RuntimeRequest::GetPluginConfig {
            plugin_id: plugin_id.to_string(),
        };
        match self.request_json(&request)? {
            PluginConfigResponse::Ok { config } => Some(config),
            PluginConfigResponse::Error { message } => {
                eprintln!("[runtime/client] get_plugin_config({plugin_id}) failed: {message}");
                None
            }
        }
    }

    pub fn set_plugin_config(&self, plugin_id: &str, config: &serde_json::Value) -> bool {
        let request = RuntimeRequest::SetPluginConfig {
            plugin_id: plugin_id.to_string(),
            config: config.clone(),
        };
        match self.request_json(&request) {
            Some(PluginConfigResponse::Ok { .. }) => true,
            Some(PluginConfigResponse::Error { message }) => {
                eprintln!("[runtime/client] set_plugin_config({plugin_id}) failed: {message}");
                false
            }
            None => false,
        }
    }

    /// Pushes a notification (toast) to the tray. One call, no async setup;
    /// the plugin id is read from the [`ENV_PLUGIN_ID`] env var. Returns
    /// `true` when the host accepted the push.
    pub fn send_notification(&self, title: &str, body: &str, level: NotificationLevel) -> bool {
        self.send_notification_with_action(title, body, level, None)
    }

    /// Pushes a notification with an optional clickable action to the tray.
    /// `action` is `(label, payload)`: the label is the button text the host
    /// renders, the payload is host-interpreted (V1: a path the host opens
    /// with the default app when the action is invoked). When `None`, the
    /// notification is plain. Returns `true` when the host accepted the push.
    pub fn send_notification_with_action(
        &self,
        title: &str,
        body: &str,
        level: NotificationLevel,
        action: Option<(&str, &str)>,
    ) -> bool {
        self.send_notification_with_layout(title, body, level, action, None, None)
    }

    pub fn send_notification_with_layout(
        &self,
        title: &str,
        body: &str,
        level: NotificationLevel,
        action: Option<(&str, &str)>,
        artifact: Option<&str>,
        layout: Option<NotificationLayout>,
    ) -> bool {
        let plugin_id = self.plugin_id_from_env();
        let request = RuntimeRequest::PushNotification {
            plugin_id,
            title: title.to_string(),
            body: body.to_string(),
            level,
            action_label: action.map(|(label, _)| label.to_string()),
            action_payload: action.map(|(_, payload)| payload.to_string()),
            artifact: artifact.map(str::to_string),
            layout,
        };
        matches!(self.request_json(&request), Some(PushAck::Handled))
    }

    /// Pushes a free-form status update to the tray. One call, no async setup;
    /// the plugin id is read from the [`ENV_PLUGIN_ID`] env var. Returns
    /// `true` when the host accepted the push.
    pub fn send_status(&self, status: &serde_json::Value) -> bool {
        let plugin_id = self.plugin_id_from_env();
        let request = RuntimeRequest::PushStatus {
            plugin_id,
            status: status.clone(),
        };
        matches!(self.request_json(&request), Some(PushAck::Handled))
    }

    fn plugin_id_from_env(&self) -> String {
        std::env::var(ENV_PLUGIN_ID).unwrap_or_default()
    }

    fn request_json<T: serde::de::DeserializeOwned>(&self, request: &RuntimeRequest) -> Option<T> {
        let mut reader = self.send(request, Some(TIMEOUT))?;
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;

        serde_json::from_str(line.trim()).ok()
    }

    pub fn subscribe(&self, events: Vec<RuntimeEventKind>) -> Option<Subscription> {
        let plugin_id = std::env::var(ENV_PLUGIN_ID).unwrap_or_else(|_| "unknown".to_string());
        self.open_subscription(RuntimeRequest::Subscribe { plugin_id, events })
    }

    pub fn lifeline(&self, plugin_id: &str) -> Option<Subscription> {
        self.open_subscription(RuntimeRequest::Lifeline {
            plugin_id: plugin_id.to_string(),
        })
    }

    fn open_subscription(&self, request: RuntimeRequest) -> Option<Subscription> {
        let mut reader = self.send(&request, None)?;
        let mut ack_line = String::new();
        reader.read_line(&mut ack_line).ok()?;

        let ack: SubscribeAck = serde_json::from_str(ack_line.trim()).ok()?;
        if !matches!(ack, SubscribeAck::Subscribed) {
            return None;
        }

        Some(Subscription { reader })
    }

    pub fn armed_lifelines(&self) -> Option<Vec<String>> {
        let mut reader = self.send(&RuntimeRequest::ArmedLifelines, Some(TIMEOUT))?;
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;

        let response: ArmedLifelinesResponse = serde_json::from_str(line.trim()).ok()?;
        Some(response.plugin_ids)
    }
}

impl Subscription {
    pub fn next_event(&mut self) -> Option<RuntimeEvent> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => serde_json::from_str(line.trim()).ok(),
        }
    }

    pub fn events(self) -> impl Iterator<Item = RuntimeEvent> {
        SubscriptionIter { sub: self }
    }
}

struct SubscriptionIter {
    sub: Subscription,
}

impl Iterator for SubscriptionIter {
    type Item = RuntimeEvent;

    fn next(&mut self) -> Option<RuntimeEvent> {
        self.sub.next_event()
    }
}
