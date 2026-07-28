use super::NotificationPlatform;

pub(super) struct Platform;

impl NotificationPlatform for Platform {
    fn send_notification(&self, _title: &str, _message: &str) -> bool {
        false
    }
}
