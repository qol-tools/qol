use qol_plugin_daemon::notification::{self, Notification, NotificationLayout, NotificationLevel};

use super::{Level, Message, Relay};

pub struct QolNotifications;

impl Relay for QolNotifications {
    fn deliver(&self, message: &Message) -> bool {
        notification::send(&notification_of(message))
    }
}

pub(super) fn notification_of(message: &Message) -> Notification {
    Notification::new(message.title.clone(), message.body.clone())
        .level(level_of(message.level))
        .key(key_of(message.level))
        .layout(NotificationLayout::banner(None))
}

fn level_of(level: Level) -> NotificationLevel {
    match level {
        Level::Info => NotificationLevel::Info,
        Level::Warn => NotificationLevel::Warn,
        Level::Error => NotificationLevel::Error,
    }
}

pub(super) fn key_of(level: Level) -> &'static str {
    match level {
        Level::Info => "notes",
        Level::Warn | Level::Error => "problems",
    }
}
