use crate::platform::macos::hid_helper::protocol::{
    FIRST_MODIFIER, LAST_MODIFIER, PAGE_APPLE_VENDOR_KEYBOARD, PAGE_APPLE_VENDOR_TOP_CASE,
    PAGE_CONSUMER, PAGE_KEYBOARD,
};
use crate::platform::macos::virtual_hid::client::request::{Keys, Request};

#[derive(Default)]
struct KeySet(Keys);

impl KeySet {
    fn set(&mut self, usage: u16, pressed: bool) {
        if pressed {
            if !self.0.contains(&usage) {
                if let Some(slot) = self.0.iter_mut().find(|slot| **slot == 0) {
                    *slot = usage;
                }
            }
        } else {
            for slot in self.0.iter_mut().filter(|slot| **slot == usage) {
                *slot = 0;
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.0.iter().all(|slot| *slot == 0)
    }
}

#[derive(Default)]
pub(crate) struct ReportState {
    modifiers: u8,
    keyboard: KeySet,
    consumer: KeySet,
    apple_keyboard: KeySet,
    top_case: KeySet,
}

impl ReportState {
    pub(crate) fn apply(&mut self, page: u16, usage: u16, pressed: bool) -> Option<Request> {
        match page {
            PAGE_KEYBOARD if (FIRST_MODIFIER..=LAST_MODIFIER).contains(&usage) => {
                let bit = 1u8 << (usage - FIRST_MODIFIER);
                if pressed {
                    self.modifiers |= bit;
                } else {
                    self.modifiers &= !bit;
                }
                Some(self.keyboard_report())
            }
            PAGE_KEYBOARD => {
                self.keyboard.set(usage, pressed);
                Some(self.keyboard_report())
            }
            PAGE_CONSUMER => {
                self.consumer.set(usage, pressed);
                Some(Request::Consumer(self.consumer.0))
            }
            PAGE_APPLE_VENDOR_KEYBOARD => {
                self.apple_keyboard.set(usage, pressed);
                Some(Request::AppleVendorKeyboard(self.apple_keyboard.0))
            }
            PAGE_APPLE_VENDOR_TOP_CASE => {
                self.top_case.set(usage, pressed);
                Some(Request::AppleVendorTopCase(self.top_case.0))
            }
            _ => None,
        }
    }

    pub(crate) fn clear(&mut self) -> Vec<Request> {
        let mut releases = Vec::new();
        if self.modifiers != 0 || !self.keyboard.is_empty() {
            releases.push(Request::Keyboard {
                modifiers: 0,
                keys: [0; 32],
            });
        }
        if !self.consumer.is_empty() {
            releases.push(Request::Consumer([0; 32]));
        }
        if !self.apple_keyboard.is_empty() {
            releases.push(Request::AppleVendorKeyboard([0; 32]));
        }
        if !self.top_case.is_empty() {
            releases.push(Request::AppleVendorTopCase([0; 32]));
        }
        *self = Self::default();
        releases
    }

    fn keyboard_report(&self) -> Request {
        Request::Keyboard {
            modifiers: self.modifiers,
            keys: self.keyboard.0,
        }
    }
}

#[derive(Default)]
pub(crate) struct PressedKeys {
    keys: Vec<(u16, u16)>,
}

impl PressedKeys {
    pub(crate) fn record(&mut self, page: u16, usage: u16, pressed: bool) {
        self.keys.retain(|key| *key != (page, usage));
        if pressed {
            self.keys.push((page, usage));
        }
    }

    pub(crate) fn drain(&mut self) -> Vec<(u16, u16)> {
        std::mem::take(&mut self.keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::macos::hid_helper::protocol::{
        PAGE_APPLE_VENDOR_TOP_CASE, PAGE_CONSUMER, PAGE_KEYBOARD,
    };

    fn keys(first: &[u16]) -> Keys {
        let mut keys = [0u16; 32];
        keys[..first.len()].copy_from_slice(first);
        keys
    }

    #[test]
    fn modifiers_go_into_the_modifier_byte() {
        let mut state = ReportState::default();
        assert_eq!(
            state.apply(PAGE_KEYBOARD, 0xE3, true),
            Some(Request::Keyboard {
                modifiers: 0x08,
                keys: keys(&[])
            })
        );
        assert_eq!(
            state.apply(PAGE_KEYBOARD, 0x06, true),
            Some(Request::Keyboard {
                modifiers: 0x08,
                keys: keys(&[0x06])
            })
        );
        assert_eq!(
            state.apply(PAGE_KEYBOARD, 0xE3, false),
            Some(Request::Keyboard {
                modifiers: 0,
                keys: keys(&[0x06])
            })
        );
    }

    #[test]
    fn a_released_key_frees_its_slot_and_duplicates_are_ignored() {
        let mut state = ReportState::default();
        state.apply(PAGE_KEYBOARD, 0x04, true);
        state.apply(PAGE_KEYBOARD, 0x04, true);
        state.apply(PAGE_KEYBOARD, 0x05, true);
        assert_eq!(
            state.apply(PAGE_KEYBOARD, 0x04, false),
            Some(Request::Keyboard {
                modifiers: 0,
                keys: keys(&[0, 0x05])
            })
        );
    }

    #[test]
    fn other_pages_use_their_own_reports() {
        let mut state = ReportState::default();
        assert_eq!(
            state.apply(PAGE_CONSUMER, 0xCD, true),
            Some(Request::Consumer(keys(&[0xCD])))
        );
        assert_eq!(
            state.apply(PAGE_APPLE_VENDOR_TOP_CASE, 0x03, true),
            Some(Request::AppleVendorTopCase(keys(&[0x03])))
        );
        assert_eq!(state.apply(0x0001, 0x80, true), None);
    }

    #[test]
    fn clear_releases_every_page_that_had_keys() {
        let mut state = ReportState::default();
        state.apply(PAGE_KEYBOARD, 0xE3, true);
        state.apply(PAGE_CONSUMER, 0xE9, true);
        assert_eq!(
            state.clear(),
            vec![
                Request::Keyboard {
                    modifiers: 0,
                    keys: keys(&[])
                },
                Request::Consumer(keys(&[])),
            ]
        );
        assert!(state.clear().is_empty());
    }

    #[test]
    fn unplugging_releases_what_the_device_held() {
        let mut pressed = PressedKeys::default();
        pressed.record(PAGE_KEYBOARD, 0x04, true);
        pressed.record(PAGE_KEYBOARD, 0xE0, true);
        pressed.record(PAGE_KEYBOARD, 0x04, false);
        pressed.record(PAGE_CONSUMER, 0xE9, true);
        assert_eq!(
            pressed.drain(),
            vec![(PAGE_KEYBOARD, 0xE0), (PAGE_CONSUMER, 0xE9)]
        );
        assert!(pressed.drain().is_empty());
    }
}
