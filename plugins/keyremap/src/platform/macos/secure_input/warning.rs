use crate::platform::macos::doctor::SecureInputHolder;
use crate::platform::macos::input::Strategy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Toast {
    pub(crate) title: String,
    pub(crate) body: String,
}

#[derive(Debug, Default)]
pub(crate) struct SecureInputWatch {
    warned_for: Option<i32>,
}

impl SecureInputWatch {
    pub(crate) fn observe(
        &mut self,
        holder: Option<&SecureInputHolder>,
        strategy: Strategy,
    ) -> Option<Toast> {
        let Some(holder) = holder else {
            self.warned_for = None;
            return None;
        };
        if self.warned_for == Some(holder.pid) {
            return None;
        }
        self.warned_for = Some(holder.pid);
        Some(toast(holder, strategy))
    }
}

pub(crate) fn toast(holder: &SecureInputHolder, strategy: Strategy) -> Toast {
    let app = &holder.app;
    match strategy {
        Strategy::EventTap => Toast {
            title: format!("Key Remap and qol hotkeys are paused: {app} has turned on Secure Input."),
            body: format!(
                "Quit {app} or turn off its secure keyboard entry to get them back. \
Installing the virtual keyboard driver keeps Key Remap working through this; \
qol-keyremap doctor shows the steps."
            ),
        },
        Strategy::VirtualHid => Toast {
            title: format!("qol hotkeys are paused: {app} has turned on Secure Input."),
            body: format!(
                "Key Remap keeps working. Quit {app} or turn off its secure keyboard entry to get qol hotkeys back."
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kitty() -> SecureInputHolder {
        SecureInputHolder {
            pid: 4242,
            app: "kitty".to_string(),
        }
    }

    fn teams() -> SecureInputHolder {
        SecureInputHolder {
            pid: 777,
            app: "Microsoft Teams".to_string(),
        }
    }

    #[test]
    fn warns_once_per_episode() {
        let mut watch = SecureInputWatch::default();
        assert!(watch.observe(Some(&kitty()), Strategy::EventTap).is_some());
        assert!(watch.observe(Some(&kitty()), Strategy::EventTap).is_none());
        assert!(watch
            .observe(Some(&kitty()), Strategy::VirtualHid)
            .is_none());
    }

    #[test]
    fn a_new_holder_starts_a_new_episode() {
        let mut watch = SecureInputWatch::default();
        watch.observe(Some(&kitty()), Strategy::EventTap);
        let toast = watch.observe(Some(&teams()), Strategy::EventTap).unwrap();
        assert!(toast.title.contains("Microsoft Teams"));
    }

    #[test]
    fn secure_input_turning_off_ends_the_episode() {
        let mut watch = SecureInputWatch::default();
        watch.observe(Some(&kitty()), Strategy::EventTap);
        assert!(watch.observe(None, Strategy::EventTap).is_none());
        assert!(watch.observe(Some(&kitty()), Strategy::EventTap).is_some());
    }

    #[test]
    fn the_event_tap_text_says_key_remap_is_paused() {
        let toast = toast(&kitty(), Strategy::EventTap);
        assert_eq!(
            toast.title,
            "Key Remap and qol hotkeys are paused: kitty has turned on Secure Input."
        );
        assert!(toast.body.contains("Quit kitty"));
        assert!(toast.body.contains("virtual keyboard driver"));
    }

    #[test]
    fn the_virtual_hid_text_says_key_remap_keeps_working() {
        let toast = toast(&kitty(), Strategy::VirtualHid);
        assert_eq!(
            toast.title,
            "qol hotkeys are paused: kitty has turned on Secure Input."
        );
        assert!(toast.body.contains("Key Remap keeps working"));
    }
}
