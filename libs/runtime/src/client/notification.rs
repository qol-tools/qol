use std::time::Duration;

use crate::protocol::{
    DaemonRequest, NotificationLayout, NotificationLevel, NotificationMeta, RuntimeRequest,
};

#[derive(Debug, Clone, Default)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub level: NotificationLevel,
    pub action: Option<(String, String)>,
    pub artifact: Option<String>,
    pub layout: Option<NotificationLayout>,
    pub activate: Option<DaemonRequest>,
    pub meta: NotificationMeta,
}

impl Notification {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            ..Self::default()
        }
    }

    pub fn level(mut self, level: NotificationLevel) -> Self {
        self.level = level;
        self
    }

    pub fn stay(mut self, duration: Duration) -> Self {
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        self.meta.duration_ms = Some(millis.max(1));
        self
    }

    pub fn until_dismissed(mut self) -> Self {
        self.meta.duration_ms = Some(0);
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.meta.key = Some(key.into());
        self
    }

    pub fn image(mut self, image: impl Into<String>) -> Self {
        self.meta.image = Some(image.into());
        self
    }

    pub fn action(mut self, label: impl Into<String>, payload: impl Into<String>) -> Self {
        self.action = Some((label.into(), payload.into()));
        self
    }

    pub fn artifact(mut self, artifact: impl Into<String>) -> Self {
        self.artifact = Some(artifact.into());
        self
    }

    pub fn layout(mut self, layout: NotificationLayout) -> Self {
        self.layout = Some(layout);
        self
    }

    pub fn activate(mut self, activate: DaemonRequest) -> Self {
        self.activate = Some(activate);
        self
    }

    pub(super) fn request(&self, plugin_id: String) -> RuntimeRequest {
        RuntimeRequest::PushNotification {
            plugin_id,
            title: self.title.clone(),
            body: self.body.clone(),
            level: self.level,
            action_label: self.action.as_ref().map(|(label, _)| label.clone()),
            action_payload: self.action.as_ref().map(|(_, payload)| payload.clone()),
            artifact: self.artifact.clone(),
            layout: self.layout.clone(),
            activate: self.activate.clone().map(Box::new),
            duration_ms: self.meta.duration_ms,
            key: self.meta.key.clone(),
            image: self.meta.image.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(notification: &Notification) -> serde_json::Value {
        serde_json::to_value(notification.request("plugin-foo".to_string())).unwrap()
    }

    #[test]
    fn a_plain_notification_carries_only_its_text_and_level() {
        let value = wire(&Notification::new("Title", "Body"));
        assert_eq!(value["cmd"], "push_notification");
        assert_eq!(value["plugin_id"], "plugin-foo");
        assert_eq!(value["title"], "Title");
        assert_eq!(value["body"], "Body");
        assert_eq!(value["level"], "info");
        for field in [
            "duration_ms",
            "key",
            "image",
            "artifact",
            "layout",
            "activate",
        ] {
            assert!(value.get(field).is_none(), "{field} in {value}");
        }
    }

    #[test]
    fn builders_map_onto_the_request_fields() {
        let layout = NotificationLayout::banner(Some("50"));
        let activate = DaemonRequest {
            action: "focus".to_string(),
            input: serde_json::json!({ "session": "kitty:42" }),
        };
        let cases = [
            (
                Notification::new("t", "b").level(NotificationLevel::Error),
                serde_json::json!({ "level": "error" }),
            ),
            (
                Notification::new("t", "b").stay(Duration::from_millis(2500)),
                serde_json::json!({ "duration_ms": 2500 }),
            ),
            (
                Notification::new("t", "b").stay(Duration::from_micros(10)),
                serde_json::json!({ "duration_ms": 1 }),
            ),
            (
                Notification::new("t", "b").stay(Duration::ZERO),
                serde_json::json!({ "duration_ms": 1 }),
            ),
            (
                Notification::new("t", "b").stay(Duration::MAX),
                serde_json::json!({ "duration_ms": u64::MAX }),
            ),
            (
                Notification::new("t", "b").until_dismissed(),
                serde_json::json!({ "duration_ms": 0 }),
            ),
            (
                Notification::new("t", "b")
                    .stay(Duration::from_secs(3))
                    .until_dismissed(),
                serde_json::json!({ "duration_ms": 0 }),
            ),
            (
                Notification::new("t", "b").key("copy"),
                serde_json::json!({ "key": "copy" }),
            ),
            (
                Notification::new("t", "b").image("/tmp/a.png"),
                serde_json::json!({ "image": "/tmp/a.png" }),
            ),
            (
                Notification::new("t", "b").action("Open", "/tmp/a"),
                serde_json::json!({ "action_label": "Open", "action_payload": "/tmp/a" }),
            ),
            (
                Notification::new("t", "b").artifact("/tmp/a.png"),
                serde_json::json!({ "artifact": "/tmp/a.png" }),
            ),
            (
                Notification::new("t", "b").layout(layout.clone()),
                serde_json::json!({ "layout": layout }),
            ),
            (
                Notification::new("t", "b").activate(activate.clone()),
                serde_json::json!({
                    "activate": { "action": "focus", "input": { "session": "kitty:42" } }
                }),
            ),
        ];
        for (notification, expected) in cases {
            let value = wire(&notification);
            for (field, expected) in expected.as_object().unwrap() {
                assert_eq!(&value[field], expected, "{field} in {value}");
            }
        }
    }

    #[test]
    fn every_builder_combined_fills_every_field() {
        let value = wire(
            &Notification::new("t", "b")
                .level(NotificationLevel::Warn)
                .stay(Duration::from_secs(5))
                .key("k")
                .image("/tmp/a.png")
                .action("Open", "/tmp/a")
                .artifact("/tmp/b.png")
                .layout(NotificationLayout::banner(None))
                .activate(DaemonRequest {
                    action: "focus".to_string(),
                    input: serde_json::Value::Null,
                }),
        );
        assert_eq!(value["level"], "warn");
        assert_eq!(value["duration_ms"], 5000);
        assert_eq!(value["key"], "k");
        assert_eq!(value["image"], "/tmp/a.png");
        assert_eq!(value["action_label"], "Open");
        assert_eq!(value["artifact"], "/tmp/b.png");
        assert_eq!(value["layout"]["style"], "banner");
        assert_eq!(value["activate"]["action"], "focus");
    }
}
