//! In-memory store of the latest pushed status per plugin id.
//!
//! The runtime socket server records every accepted `PushStatus` here; the
//! dashboard HTTP API reads the snapshot so the web UI can show a persistent
//! per-plugin status instead of a one-shot toast. The registry is process-wide
//! (same pattern as the runtime publisher) because the socket thread that
//! receives pushes and the axum task that serves the dashboard share no
//! parent object.
//!
//! Every stored status is also forwarded to the daemon event bus as a
//! `status_changed` event, so the dashboard can update the card live over SSE
//! instead of waiting for the next refresh.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::daemon::DaemonEvent;

static REGISTRY: OnceLock<PluginStatusRegistry> = OnceLock::new();

/// Keeps the latest `PushStatus` payload per plugin id.
#[derive(Default)]
pub(crate) struct PluginStatusRegistry {
    statuses: Mutex<HashMap<String, serde_json::Value>>,
}

impl PluginStatusRegistry {
    /// The process-wide registry, created on first use.
    pub(crate) fn shared() -> &'static Self {
        REGISTRY.get_or_init(PluginStatusRegistry::default)
    }

    /// Records the latest status for `plugin_id`, replacing any previous one,
    /// and broadcasts it to the daemon event bus when one is installed.
    pub(crate) fn set(&self, plugin_id: &str, status: serde_json::Value) {
        let Ok(mut statuses) = self.statuses.lock() else {
            return;
        };
        statuses.insert(plugin_id.to_string(), status.clone());
        if let Some(events) = super::super::publisher::events() {
            events.send(DaemonEvent::StatusChanged {
                plugin_id: plugin_id.to_string(),
                status,
            });
        }
    }

    pub(crate) fn clear(&self, plugin_id: &str) {
        let Ok(mut statuses) = self.statuses.lock() else {
            return;
        };
        if statuses.remove(plugin_id).is_none() {
            return;
        }
        drop(statuses);
        if let Some(events) = super::super::publisher::events() {
            events.send(DaemonEvent::StatusChanged {
                plugin_id: plugin_id.to_string(),
                status: serde_json::Value::Null,
            });
        }
    }

    /// A copy of every plugin id mapped to its latest status.
    pub(crate) fn snapshot(&self) -> HashMap<String, serde_json::Value> {
        let Ok(statuses) = self.statuses.lock() else {
            return HashMap::new();
        };
        statuses.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_empty_before_any_push() {
        let registry = PluginStatusRegistry::default();
        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn set_records_the_latest_status_per_plugin() {
        let registry = PluginStatusRegistry::default();
        registry.set(
            "plugin-qol-shot",
            serde_json::json!({ "state": "recording" }),
        );
        registry.set("plugin-cli-sessions", serde_json::json!({ "attention": 1 }));
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(
            snapshot["plugin-qol-shot"],
            serde_json::json!({ "state": "recording" })
        );
        assert_eq!(
            snapshot["plugin-cli-sessions"],
            serde_json::json!({ "attention": 1 })
        );
    }

    #[test]
    fn set_replaces_a_previous_status_for_the_same_plugin() {
        let registry = PluginStatusRegistry::default();
        registry.set(
            "plugin-qol-shot",
            serde_json::json!({ "state": "recording" }),
        );
        registry.set(
            "plugin-qol-shot",
            serde_json::json!({ "state": "saved", "file": "a.png" }),
        );
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(
            snapshot["plugin-qol-shot"],
            serde_json::json!({ "state": "saved", "file": "a.png" })
        );
    }

    #[test]
    fn clear_forgets_only_the_named_plugin() {
        let registry = PluginStatusRegistry::default();
        registry.set(
            "plugin-qol-shot",
            serde_json::json!({ "state": "recording" }),
        );
        registry.set("plugin-cli-sessions", serde_json::json!({ "attention": 1 }));

        registry.clear("plugin-qol-shot");
        registry.clear("plugin-not-pushed-anything");

        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(
            snapshot["plugin-cli-sessions"],
            serde_json::json!({ "attention": 1 })
        );
    }

    #[test]
    fn set_broadcasts_status_changed_to_the_event_bus() {
        use std::sync::Arc;
        use std::time::Duration;

        let events = Arc::new(crate::daemon::EventBus::new());
        let mut rx = events.subscribe();
        super::super::super::publisher::install_events(events);

        let registry = PluginStatusRegistry::default();
        registry.set(
            "plugin-push-status-event-bus",
            serde_json::json!({ "state": "recording" }),
        );

        // The bus is process-wide, so other tests may interleave their own
        // pushes; skip those and wait for the one this test recorded.
        for _ in 0..100 {
            match rx.try_recv() {
                Ok(event) => {
                    let DaemonEvent::StatusChanged { plugin_id, status } = event else {
                        continue;
                    };
                    if plugin_id == "plugin-push-status-event-bus" {
                        assert_eq!(status, serde_json::json!({ "state": "recording" }));
                        return;
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        panic!("timed out waiting for status_changed event");
    }
}
