use super::evdev_backend::{emit_key_ups, keycode_name, TrackedVirtualDevice};
use evdev::raw_stream::RawDevice;
use evdev::{AttributeSet, KeyCode};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

pub(super) fn physical_down_union(keyboards: &[(PathBuf, RawDevice)]) -> HashSet<u16> {
    let mut down = HashSet::new();
    for (_, device) in keyboards {
        for key in device.get_key_state().unwrap_or_default().iter() {
            down.insert(key.0);
        }
    }
    down
}

pub(super) fn stuck_candidates(x_down: &HashSet<u16>, physical_down: &HashSet<u16>) -> Vec<u16> {
    let mut candidates: Vec<u16> = x_down.difference(physical_down).copied().collect();
    candidates.sort_unstable();
    candidates
}

pub(super) fn healable_candidates(
    candidates: &[u16],
    declared_keys: &AttributeSet<KeyCode>,
) -> (Vec<u16>, Vec<u16>) {
    let mut healable = Vec::new();
    let mut unhealable = Vec::new();
    for &code in candidates {
        if declared_keys.contains(KeyCode::new(code)) {
            healable.push(code);
        } else {
            unhealable.push(code);
        }
    }
    (healable, unhealable)
}

pub(super) fn heal_stuck_keys(
    virtual_device: &Mutex<TrackedVirtualDevice>,
    physical_down: &HashSet<u16>,
    declared_keys: &AttributeSet<KeyCode>,
) {
    if crate::desktop_state::is_wayland() {
        log::info!(
            "evdev: skipping stuck-key heal on a Wayland session; the X server (XWayland) core keymap only reflects events delivered to X clients and is empty or stale whenever a Wayland-native window has focus, so its bits are not trustworthy stuck-key candidates"
        );
        return;
    }
    let x_down = crate::hotkeys::platform::PhysicalHotkeyState::connect()
        .ok()
        .and_then(|state| state.snapshot().ok())
        .map(|snapshot| snapshot.down_evdev_codes());
    let Some(x_down) = x_down else {
        log::info!("evdev: X core keymap healing is unavailable; skipping stuck-key heal");
        return;
    };
    let candidates = stuck_candidates(&x_down, physical_down);
    if candidates.is_empty() {
        return;
    }
    let (healable, unhealable) = healable_candidates(&candidates, declared_keys);
    if !healable.is_empty() {
        let names = healable
            .iter()
            .map(|code| keycode_name(*code))
            .collect::<Vec<_>>()
            .join(",");
        log::warn!(
            "evdev: healing {} stuck key(s) latched in the X server: {names}",
            healable.len()
        );
        emit_key_ups(virtual_device, healable);
    }
    if !unhealable.is_empty() {
        let names = unhealable
            .iter()
            .map(|code| keycode_name(*code))
            .collect::<Vec<_>>()
            .join(",");
        log::warn!(
            "evdev: {} stuck key(s) latched in the X server cannot be healed because the virtual keyboard does not declare them (their keyboard was unplugged or belongs to a skipped device class): {names}",
            unhealable.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healable_candidates_split_on_declared_key_caps() {
        let declared = AttributeSet::from_iter([KeyCode::KEY_A, KeyCode::KEY_LEFTCTRL]);
        let candidates = [
            KeyCode::KEY_A.0,
            KeyCode::KEY_RIGHTCTRL.0,
            KeyCode::KEY_LEFTCTRL.0,
            KeyCode::KEY_C.0,
        ];
        let (healable, unhealable) = healable_candidates(&candidates, &declared);
        assert_eq!(healable, vec![KeyCode::KEY_A.0, KeyCode::KEY_LEFTCTRL.0]);
        assert_eq!(unhealable, vec![KeyCode::KEY_RIGHTCTRL.0, KeyCode::KEY_C.0]);
    }

    #[test]
    fn healable_candidates_preserve_candidate_order_within_each_side() {
        let declared = AttributeSet::from_iter([KeyCode::KEY_Z, KeyCode::KEY_B]);
        let candidates = [
            KeyCode::KEY_C.0,
            KeyCode::KEY_Z.0,
            KeyCode::KEY_A.0,
            KeyCode::KEY_B.0,
        ];
        let (healable, unhealable) = healable_candidates(&candidates, &declared);
        assert_eq!(healable, vec![KeyCode::KEY_Z.0, KeyCode::KEY_B.0]);
        assert_eq!(unhealable, vec![KeyCode::KEY_C.0, KeyCode::KEY_A.0]);
    }

    #[test]
    fn healable_candidates_with_no_declared_keys_unheal_everything() {
        let declared = AttributeSet::new();
        let candidates = [KeyCode::KEY_A.0, KeyCode::KEY_DOWN.0];
        let (healable, unhealable) = healable_candidates(&candidates, &declared);
        assert!(healable.is_empty());
        assert_eq!(unhealable, candidates.to_vec());
    }

    #[test]
    fn healable_candidates_with_everything_declared_unheal_nothing() {
        let declared = AttributeSet::from_iter([KeyCode::KEY_A, KeyCode::KEY_DOWN]);
        let candidates = [KeyCode::KEY_A.0, KeyCode::KEY_DOWN.0];
        let (healable, unhealable) = healable_candidates(&candidates, &declared);
        assert_eq!(healable, candidates.to_vec());
        assert!(unhealable.is_empty());
    }

    #[test]
    fn healable_candidates_on_empty_candidates_are_both_empty() {
        let declared = AttributeSet::from_iter([KeyCode::KEY_A]);
        let (healable, unhealable) = healable_candidates(&[], &declared);
        assert!(healable.is_empty());
        assert!(unhealable.is_empty());
    }
}
