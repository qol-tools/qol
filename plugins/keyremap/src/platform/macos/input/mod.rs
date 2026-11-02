pub(crate) mod backends;
pub(crate) mod selection;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use qol_runtime::keyremap_marker;

use crate::platform::engine::remap::Modifiers;

const MARKER_GRACE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendId {
    EventTap,
    HidHelper,
    KarabinerElements,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Capabilities {
    pub(crate) survives_secure_input: bool,
    pub(crate) owns_key_events: bool,
}

impl BackendId {
    const ALL: [Self; 3] = [Self::EventTap, Self::HidHelper, Self::KarabinerElements];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::EventTap => "event_tap",
            Self::HidHelper => "hid_helper",
            Self::KarabinerElements => "karabiner_elements",
        }
    }

    pub(crate) fn capabilities(self) -> Capabilities {
        match self {
            Self::EventTap => Capabilities {
                survives_secure_input: false,
                owns_key_events: false,
            },
            Self::HidHelper | Self::KarabinerElements => Capabilities {
                survives_secure_input: true,
                owns_key_events: true,
            },
        }
    }
}

#[derive(Default)]
pub(crate) struct BackendCell(AtomicU8);

impl BackendCell {
    pub(crate) fn get(&self) -> BackendId {
        Self::decode(self.0.load(Ordering::SeqCst))
    }

    pub(crate) fn set(&self, backend: BackendId) -> BackendId {
        Self::decode(self.0.swap(Self::encode(backend), Ordering::SeqCst))
    }

    pub(crate) fn set_from_helper(&self, backend: BackendId) -> BackendId {
        let karabiner = Self::encode(BackendId::KarabinerElements);
        let wanted = Self::encode(backend);
        let previous = self
            .0
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |raw| {
                (raw != karabiner).then_some(wanted)
            })
            .unwrap_or_else(|raw| raw);
        Self::decode(previous)
    }

    fn encode(backend: BackendId) -> u8 {
        BackendId::ALL
            .iter()
            .position(|id| *id == backend)
            .and_then(|index| u8::try_from(index).ok())
            .unwrap_or_default()
    }

    fn decode(raw: u8) -> BackendId {
        BackendId::ALL
            .get(usize::from(raw))
            .copied()
            .unwrap_or(BackendId::EventTap)
    }
}

#[derive(Default)]
pub(crate) struct MarkerBook {
    entries: Mutex<HashMap<u16, (i64, Option<Instant>)>>,
}

impl MarkerBook {
    pub(crate) fn insert(&self, keycode: u16, marker: i64) {
        self.entries().insert(keycode, (marker, None));
    }

    pub(crate) fn release(&self, keycode: u16, now: Instant) {
        if let Some(entry) = self.entries().get_mut(&keycode) {
            entry.1 = Some(now);
        }
    }

    pub(crate) fn lookup(&self, keycode: u16, key_down: bool, now: Instant) -> Option<i64> {
        let mut entries = self.entries();
        entries.retain(|_, (_, released)| {
            released.is_none_or(|at| now.saturating_duration_since(at) <= MARKER_GRACE)
        });
        let (marker, released) = entries.get(&keycode)?;
        (!key_down || released.is_none()).then_some(*marker)
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<u16, (i64, Option<Instant>)>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) struct InputState {
    pub(crate) backend: BackendCell,
    pub(crate) markers: MarkerBook,
    helper_allowed: AtomicBool,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            backend: BackendCell::default(),
            markers: MarkerBook::default(),
            helper_allowed: AtomicBool::new(true),
        }
    }
}

impl InputState {
    pub(crate) fn helper_allowed(&self) -> bool {
        self.helper_allowed.load(Ordering::SeqCst)
    }

    pub(crate) fn allow_helper(&self, allowed: bool) {
        self.helper_allowed.store(allowed, Ordering::SeqCst);
    }
}

pub(crate) fn marker_for(mods: Modifiers, key: u16) -> i64 {
    let mut bits = 0;
    if mods.ctrl {
        bits |= keyremap_marker::MOD_CTRL;
    }
    if mods.shift {
        bits |= keyremap_marker::MOD_SHIFT;
    }
    if mods.alt {
        bits |= keyremap_marker::MOD_ALT;
    }
    if mods.cmd {
        bits |= keyremap_marker::MOD_SUPER;
    }
    keyremap_marker::encode(bits, key)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn the_backend_starts_on_the_event_tap_and_reports_the_previous_one() {
        let cell = BackendCell::default();
        assert_eq!(cell.get(), BackendId::EventTap);
        for backend in BackendId::ALL {
            let previous = cell.get();
            assert_eq!(cell.set(backend), previous);
            assert_eq!(cell.get(), backend);
        }
    }

    #[test]
    fn the_keyboard_helper_never_takes_over_from_karabiner() {
        let cell = BackendCell::default();
        assert_eq!(
            cell.set_from_helper(BackendId::HidHelper),
            BackendId::EventTap
        );
        assert_eq!(cell.get(), BackendId::HidHelper);
        cell.set(BackendId::KarabinerElements);
        assert_eq!(
            cell.set_from_helper(BackendId::EventTap),
            BackendId::KarabinerElements
        );
        assert_eq!(cell.get(), BackendId::KarabinerElements);
    }

    #[test]
    fn a_marker_covers_the_held_key_and_its_late_key_up() {
        let book = MarkerBook::default();
        let start = Instant::now();
        book.insert(0x08, 42);
        assert_eq!(book.lookup(0x08, true, start), Some(42));
        book.release(0x08, start);
        assert_eq!(
            book.lookup(0x08, true, start),
            None,
            "a new key down is not the remapped one"
        );
        assert_eq!(book.lookup(0x08, false, start + MARKER_GRACE), Some(42));
        assert_eq!(
            book.lookup(0x08, false, start + MARKER_GRACE + Duration::from_millis(1)),
            None
        );
        assert_eq!(book.lookup(0x09, true, start), None);
    }

    #[test]
    fn markers_encode_the_combo_the_user_pressed() {
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        let decoded = qol_runtime::keyremap_marker::decode(marker_for(ctrl, 0x08)).unwrap();
        assert_eq!(decoded.mods, qol_runtime::keyremap_marker::MOD_CTRL);
        assert_eq!(decoded.key, 0x08);

        let right_option = Modifiers {
            alt: true,
            ralt: true,
            ..Modifiers::NONE
        };
        let decoded = qol_runtime::keyremap_marker::decode(marker_for(right_option, 0x13)).unwrap();
        assert_eq!(decoded.mods, qol_runtime::keyremap_marker::MOD_ALT);
    }
}
