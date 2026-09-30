pub(crate) mod backends;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use qol_runtime::keyremap_marker;

use super::app::remap::Modifiers;

const MARKER_GRACE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Strategy {
    EventTap,
    VirtualHid,
}

#[derive(Default)]
pub(crate) struct StrategyCell(AtomicU8);

impl StrategyCell {
    pub(crate) fn get(&self) -> Strategy {
        Self::decode(self.0.load(Ordering::SeqCst))
    }

    pub(crate) fn set(&self, strategy: Strategy) -> Strategy {
        Self::decode(
            self.0
                .swap(u8::from(strategy == Strategy::VirtualHid), Ordering::SeqCst),
        )
    }

    fn decode(raw: u8) -> Strategy {
        if raw == 1 {
            Strategy::VirtualHid
        } else {
            Strategy::EventTap
        }
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

#[derive(Default)]
pub(crate) struct InputState {
    pub(crate) strategy: StrategyCell,
    pub(crate) markers: MarkerBook,
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
    fn the_strategy_starts_on_the_event_tap_and_reports_the_previous_one() {
        let cell = StrategyCell::default();
        assert_eq!(cell.get(), Strategy::EventTap);
        assert_eq!(cell.set(Strategy::VirtualHid), Strategy::EventTap);
        assert_eq!(cell.get(), Strategy::VirtualHid);
        assert_eq!(cell.set(Strategy::EventTap), Strategy::VirtualHid);
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
