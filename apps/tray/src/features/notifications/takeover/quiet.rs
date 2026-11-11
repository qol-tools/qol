use qol_plugin_daemon::notification::platform::{self, NotificationInhibit};

use super::state::{Health, Mechanism};

pub(super) const BANNER_SWITCH: &str = "desktop banner switch";
const INHIBIT: &str = "notification inhibit";
const BANNERS_BACK_ON: &str = "the desktop is showing banners again";

#[derive(Default)]
pub(super) struct Quiet {
    inhibit: Option<NotificationInhibit>,
    banners_turned_off: bool,
}

impl Quiet {
    pub(super) fn apply(&mut self, wanted: bool, enforce: bool) -> Vec<Mechanism> {
        if !wanted {
            self.release();
            return Vec::new();
        }
        if enforce {
            platform::set_os_banners(false);
            self.banners_turned_off = true;
            if self.inhibit.is_none() {
                self.inhibit = platform::acquire_inhibit();
            }
        }
        let mut mechanisms = Vec::new();
        match platform::os_banners_showing() {
            Some(false) => mechanisms.push(Mechanism::hiding(BANNER_SWITCH, Health::Active)),
            Some(true) => mechanisms.push(Mechanism::hiding(
                BANNER_SWITCH,
                Health::Failed(BANNERS_BACK_ON.to_string()),
            )),
            None => {}
        }
        if self.inhibit.is_some() {
            mechanisms.push(Mechanism::hiding(INHIBIT, Health::Active));
        }
        mechanisms
    }

    fn release(&mut self) {
        if self.banners_turned_off {
            platform::set_os_banners(true);
            self.banners_turned_off = false;
        }
        self.inhibit = None;
    }
}
