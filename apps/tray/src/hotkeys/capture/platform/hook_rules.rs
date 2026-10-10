use super::key_matcher::KeyTransition;
use qol_hotkeys::grammar::Modifier as Mod;
use std::collections::{BTreeSet, HashSet};

const VK_LSHIFT: u16 = 0xA0;
const VK_RSHIFT: u16 = 0xA1;
const VK_LCONTROL: u16 = 0xA2;
const VK_RCONTROL: u16 = 0xA3;
const VK_LMENU: u16 = 0xA4;
const VK_RMENU: u16 = 0xA5;
const VK_LWIN: u16 = 0x5B;
const VK_RWIN: u16 = 0x5C;

const ALTGR_SCAN_FLAG: u32 = 0x200;
const REPEAT_WINDOW_MS: u32 = 1_100;
const REARM_SLACK_MS: u32 = 500;
const SHIFT_STATE_SHIFT: u16 = 0x01;
const SHIFT_STATE_CTRL: u16 = 0x02;
const SHIFT_STATE_ALT: u16 = 0x04;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyEvent {
    pub(crate) key: u16,
    pub(crate) scan: u32,
    pub(crate) down: bool,
    pub(crate) time: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyFacts {
    pub(crate) down: bool,
    pub(crate) tracked: bool,
    pub(crate) swallowed: bool,
    pub(crate) async_down: bool,
    pub(crate) repeats_last_down: bool,
}

pub(crate) fn transition(facts: KeyFacts) -> KeyTransition {
    if !facts.down {
        return KeyTransition::Release;
    }
    if !facts.tracked {
        return KeyTransition::Press;
    }
    let still_held = if facts.swallowed {
        facts.repeats_last_down
    } else {
        facts.async_down
    };
    if still_held {
        KeyTransition::Repeat
    } else {
        KeyTransition::Press
    }
}

pub(crate) fn needs_menu_mask(
    transition: KeyTransition,
    swallow: bool,
    mods: &BTreeSet<Mod>,
) -> bool {
    transition == KeyTransition::Press
        && swallow
        && (mods.contains(&Mod::Super) || mods.contains(&Mod::Alt))
}

pub(crate) fn needs_rearm(last_input_tick: u32, last_hook_tick: u32) -> bool {
    let gap = last_input_tick.wrapping_sub(last_hook_tick);
    gap > REARM_SLACK_MS && gap < u32::MAX / 2
}

pub(crate) fn held_mods(is_down: impl Fn(u16) -> bool, synthetic_ctrl: bool) -> BTreeSet<Mod> {
    let mut mods = BTreeSet::new();
    if is_down(VK_LSHIFT) || is_down(VK_RSHIFT) {
        mods.insert(Mod::Shift);
    }
    if is_down(VK_RCONTROL) || (is_down(VK_LCONTROL) && !synthetic_ctrl) {
        mods.insert(Mod::Ctrl);
    }
    if is_down(VK_LMENU) || is_down(VK_RMENU) {
        mods.insert(Mod::Alt);
    }
    if is_down(VK_LWIN) || is_down(VK_RWIN) {
        mods.insert(Mod::Super);
    }
    mods
}

pub(crate) fn decode_vk_scan(scan: i16) -> Option<(u16, BTreeSet<Mod>)> {
    if scan == -1 {
        return None;
    }
    let bits = scan as u16;
    let shift_state = bits >> 8;
    if shift_state & !(SHIFT_STATE_SHIFT | SHIFT_STATE_CTRL | SHIFT_STATE_ALT) != 0 {
        return None;
    }
    let mut mods = BTreeSet::new();
    if shift_state & SHIFT_STATE_SHIFT != 0 {
        mods.insert(Mod::Shift);
    }
    if shift_state & SHIFT_STATE_ALT != 0 {
        mods.insert(Mod::Alt);
    } else if shift_state & SHIFT_STATE_CTRL != 0 {
        mods.insert(Mod::Ctrl);
    }
    Some((bits & 0xFF, mods))
}

#[derive(Debug, Default)]
pub(crate) struct KeyHistory {
    held: HashSet<u16>,
    last_down: Option<(u16, u32)>,
    synthetic_ctrl: bool,
}

impl KeyHistory {
    pub(crate) fn synthetic_ctrl(&self) -> bool {
        self.synthetic_ctrl
    }

    pub(crate) fn note_altgr(&mut self, event: KeyEvent) -> bool {
        if event.key == VK_LCONTROL && event.scan & ALTGR_SCAN_FLAG != 0 {
            self.synthetic_ctrl = event.down;
            return true;
        }
        if event.key == VK_RMENU && !event.down {
            self.synthetic_ctrl = false;
        }
        false
    }

    pub(crate) fn observe(
        &mut self,
        event: KeyEvent,
        swallowed: bool,
        async_down: bool,
    ) -> KeyTransition {
        let facts = KeyFacts {
            down: event.down,
            tracked: self.held.contains(&event.key),
            swallowed,
            async_down,
            repeats_last_down: self.last_down.is_some_and(|(key, time)| {
                key == event.key && event.time.wrapping_sub(time) <= REPEAT_WINDOW_MS
            }),
        };
        if event.down {
            self.held.insert(event.key);
            self.last_down = Some((event.key, event.time));
        } else {
            self.held.remove(&event.key);
            if self.last_down.is_some_and(|(key, _)| key == event.key) {
                self.last_down = None;
            }
        }
        transition(facts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u16 = 0x52;
    const T: u16 = 0x54;

    fn facts(
        down: bool,
        tracked: bool,
        swallowed: bool,
        async_down: bool,
        repeats_last_down: bool,
    ) -> KeyFacts {
        KeyFacts {
            down,
            tracked,
            swallowed,
            async_down,
            repeats_last_down,
        }
    }

    fn event(key: u16, down: bool, time: u32) -> KeyEvent {
        KeyEvent {
            key,
            scan: 0,
            down,
            time,
        }
    }

    fn mods(list: &[Mod]) -> BTreeSet<Mod> {
        list.iter().copied().collect()
    }

    #[test]
    fn transition_cases() {
        use KeyTransition::{Press, Release, Repeat};
        let cases: &[(KeyFacts, KeyTransition, &str)] = &[
            (facts(false, true, true, true, true), Release, "tracked up"),
            (
                facts(false, false, false, false, false),
                Release,
                "untracked up",
            ),
            (
                facts(true, false, true, true, true),
                Press,
                "untracked down",
            ),
            (
                facts(true, true, false, true, false),
                Repeat,
                "passed key still down",
            ),
            (
                facts(true, true, false, false, true),
                Press,
                "passed key, lost up",
            ),
            (
                facts(true, true, true, false, true),
                Repeat,
                "swallowed key repeats",
            ),
            (
                facts(true, true, true, true, false),
                Press,
                "swallowed key, lost up",
            ),
        ];
        for (input, expected, label) in cases {
            assert_eq!(transition(*input), *expected, "{label}");
        }
    }

    #[test]
    fn needs_menu_mask_cases() {
        use KeyTransition::{Press, Release, Repeat};
        let cases: &[(KeyTransition, bool, &[Mod], bool)] = &[
            (Press, true, &[Mod::Super], true),
            (Press, true, &[Mod::Alt], true),
            (Press, true, &[Mod::Ctrl, Mod::Shift], false),
            (Press, false, &[Mod::Super], false),
            (Repeat, true, &[Mod::Super], false),
            (Release, true, &[Mod::Alt], false),
        ];
        for (transition, swallow, held, expected) in cases {
            assert_eq!(
                needs_menu_mask(*transition, *swallow, &mods(held)),
                *expected,
                "{transition:?} swallow={swallow} mods={held:?}"
            );
        }
    }

    #[test]
    fn needs_rearm_cases() {
        let cases: &[(u32, u32, bool)] = &[
            (10_000, 10_000, false),
            (10_400, 10_000, false),
            (11_000, 10_000, true),
            (9_000, 10_000, false),
            (200, u32::MAX - 1_000, true),
            (u32::MAX - 1_000, 200, false),
        ];
        for (input, hook, expected) in cases {
            assert_eq!(
                needs_rearm(*input, *hook),
                *expected,
                "input={input} hook={hook}"
            );
        }
    }

    #[test]
    fn held_mods_ignore_the_synthetic_ctrl_of_altgr() {
        let down = |keys: &'static [u16]| move |key: u16| keys.contains(&key);
        assert_eq!(
            held_mods(down(&[VK_LCONTROL, VK_RMENU]), true),
            mods(&[Mod::Alt])
        );
        assert_eq!(
            held_mods(down(&[VK_LCONTROL, VK_RMENU]), false),
            mods(&[Mod::Ctrl, Mod::Alt])
        );
        assert_eq!(
            held_mods(down(&[VK_RCONTROL, VK_RMENU]), true),
            mods(&[Mod::Ctrl, Mod::Alt])
        );
        assert_eq!(
            held_mods(down(&[VK_RSHIFT, VK_LWIN]), false),
            mods(&[Mod::Shift, Mod::Super])
        );
    }

    #[test]
    fn altgr_marks_and_clears_the_synthetic_ctrl() {
        let mut history = KeyHistory::default();
        let fake_ctrl = KeyEvent {
            key: VK_LCONTROL,
            scan: 0x21D,
            down: true,
            time: 0,
        };
        assert!(history.note_altgr(fake_ctrl));
        assert!(history.synthetic_ctrl());
        assert!(!history.note_altgr(event(VK_RMENU, false, 1)));
        assert!(!history.synthetic_ctrl());
        assert!(!history.note_altgr(KeyEvent {
            scan: 0x1D,
            ..fake_ctrl
        }));
        assert!(!history.synthetic_ctrl());
    }

    type DecodeCase = (i16, Option<(u16, &'static [Mod])>, &'static str);

    #[test]
    fn decode_vk_scan_cases() {
        let cases: &[DecodeCase] = &[
            (0x0131, Some((0x31, &[Mod::Shift])), "'!' on US is Shift+1"),
            (0x00BF, Some((0xBF, &[])), "'/' on US needs no modifier"),
            (
                0x0651,
                Some((0x51, &[Mod::Alt])),
                "'@' on German is AltGr+Q",
            ),
            (
                0x0731,
                Some((0x31, &[Mod::Shift, Mod::Alt])),
                "shift plus AltGr",
            ),
            (0x0231, Some((0x31, &[Mod::Ctrl])), "ctrl only"),
            (0x0831, None, "kana shift states are not reachable"),
            (-1, None, "no key produces the character"),
        ];
        for (scan, expected, label) in cases {
            let expected = expected.map(|(key, held)| (key, mods(held)));
            assert_eq!(decode_vk_scan(*scan), expected, "{label}");
        }
    }

    #[test]
    fn a_swallowed_key_auto_repeats_until_another_key_goes_down() {
        use KeyTransition::{Press, Repeat};
        let mut history = KeyHistory::default();
        assert_eq!(history.observe(event(R, true, 0), false, false), Press);
        assert_eq!(history.observe(event(R, true, 500), true, false), Repeat);
        assert_eq!(history.observe(event(R, true, 530), true, false), Repeat);
        assert_eq!(history.observe(event(T, true, 600), false, false), Press);
        assert_eq!(
            history.observe(event(R, true, 5_000), true, false),
            Press,
            "a lost key-up must not eat the next press"
        );
    }

    #[test]
    fn a_lost_key_up_heals_on_the_next_press_after_a_pause() {
        use KeyTransition::{Press, Release};
        let mut history = KeyHistory::default();
        assert_eq!(history.observe(event(R, true, 0), false, false), Press);
        assert_eq!(history.observe(event(R, true, 10_000), true, false), Press);
        assert_eq!(
            history.observe(event(R, false, 10_100), true, false),
            Release
        );
        assert_eq!(history.observe(event(R, true, 10_200), false, false), Press);
    }
}
