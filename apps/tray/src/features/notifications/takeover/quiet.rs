use qol_plugin_daemon::notification::{
    acquire_inhibit, os_banners_showing, set_os_banners, NotificationInhibit,
};

use super::state::{Health, Mechanism};

pub(super) const BANNER_SWITCH: &str = "desktop banner switch";
const INHIBIT: &str = "notification inhibit";
const BANNERS_STAY_ON: &str = "the desktop keeps showing banners";
const BANNER_SWITCH_UNREADABLE: &str = "the desktop banner switch can no longer be read";

#[derive(Default)]
pub(super) struct Quiet {
    inhibit: Option<NotificationInhibit>,
    holds_banner_switch: bool,
}

impl Quiet {
    pub(super) fn apply(&mut self, wanted: bool) -> Vec<Mechanism> {
        if !wanted {
            self.release();
            return Vec::new();
        }
        if self.inhibit.is_none() {
            self.inhibit = acquire_inhibit();
        }
        let mut mechanisms = Vec::new();
        if let Some(health) = self.hide_banners() {
            mechanisms.push(Mechanism::hiding(BANNER_SWITCH, health));
        }
        if self.inhibit.is_some() {
            mechanisms.push(Mechanism::hiding(INHIBIT, Health::Active));
        }
        mechanisms
    }

    fn hide_banners(&mut self) -> Option<Health> {
        let showing = os_banners_showing()?;
        self.holds_banner_switch = true;
        if !showing {
            return Some(Health::Active);
        }
        set_os_banners(false);
        Some(banner_switch_health(os_banners_showing()))
    }

    fn release(&mut self) {
        if self.holds_banner_switch {
            set_os_banners(true);
            self.holds_banner_switch = false;
        }
        self.inhibit = None;
    }
}

fn banner_switch_health(showing_after_turning_off: Option<bool>) -> Health {
    match showing_after_turning_off {
        Some(false) => Health::Active,
        Some(true) => Health::Failed(BANNERS_STAY_ON.to_string()),
        None => Health::Failed(BANNER_SWITCH_UNREADABLE.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_that_will_not_turn_off_is_reported() {
        assert_eq!(banner_switch_health(Some(false)), Health::Active);
        assert_eq!(
            banner_switch_health(Some(true)),
            Health::Failed(BANNERS_STAY_ON.to_string())
        );
        assert_eq!(
            banner_switch_health(None),
            Health::Failed(BANNER_SWITCH_UNREADABLE.to_string())
        );
    }
}
