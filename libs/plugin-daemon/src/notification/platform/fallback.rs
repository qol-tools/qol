use super::NotificationPlatform;

pub(super) struct Platform;

impl NotificationPlatform for Platform {
    fn send_notification(&self, _title: &str, _message: &str) -> bool {
        false
    }

    fn os_do_not_disturb(&self) -> Option<bool> {
        None
    }

    fn set_os_banners(&self, _showing: bool) {}

    fn acquire_inhibit(&self) -> Option<NotificationInhibit> {
        None
    }
}

pub struct NotificationInhibit;
