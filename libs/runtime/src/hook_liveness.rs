use std::sync::atomic::{AtomicU32, Ordering};

const UNSEEN_SLACK_MS: u32 = 500;

#[derive(Debug, Default)]
pub struct SeenInput {
    keyboard: AtomicU32,
    mouse: AtomicU32,
}

impl SeenInput {
    pub const fn new() -> Self {
        Self {
            keyboard: AtomicU32::new(0),
            mouse: AtomicU32::new(0),
        }
    }

    pub fn keyboard(&self, tick: u32) {
        self.keyboard.store(tick, Ordering::Relaxed);
    }

    pub fn mouse(&self, tick: u32) {
        self.mouse.store(tick, Ordering::Relaxed);
    }

    pub fn rearmed(&self, tick: u32) {
        self.keyboard(tick);
        self.mouse(tick);
    }

    pub fn keyboard_tick(&self) -> u32 {
        self.keyboard.load(Ordering::Relaxed)
    }

    pub fn unseen(&self, last_input_tick: u32) -> bool {
        input_unseen(
            last_input_tick,
            &[
                self.keyboard.load(Ordering::Relaxed),
                self.mouse.load(Ordering::Relaxed),
            ],
        )
    }
}

pub fn input_unseen(last_input_tick: u32, seen_ticks: &[u32]) -> bool {
    seen_ticks.iter().all(|seen| {
        let gap = last_input_tick.wrapping_sub(*seen);
        gap > UNSEEN_SLACK_MS && gap < u32::MAX / 2
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_newer_than_every_hook_by_more_than_the_slack_is_unseen() {
        let cases: &[(u32, &[u32], bool)] = &[
            (10_000, &[10_000], false),
            (10_400, &[10_000], false),
            (11_000, &[10_000], true),
            (9_000, &[10_000], false),
            (200, &[u32::MAX - 1_000], true),
            (u32::MAX - 1_000, &[200], false),
            (11_000, &[10_000, 10_900], false),
            (11_000, &[10_000, 10_100], true),
            (11_000, &[], true),
        ];
        for (input, seen, expected) in cases {
            assert_eq!(
                input_unseen(*input, seen),
                *expected,
                "input={input} seen={seen:?}"
            );
        }
    }

    #[test]
    fn mouse_moves_for_thirty_seconds_never_count_as_unseen() {
        let seen = SeenInput::new();
        seen.rearmed(1_000);
        seen.keyboard(1_000);
        for tick in (1_000..31_000).step_by(16) {
            seen.mouse(tick);
            assert!(!seen.unseen(tick), "tick={tick}");
        }
    }

    #[test]
    fn keys_the_keyboard_hook_missed_after_mouse_use_are_unseen() {
        let seen = SeenInput::new();
        seen.keyboard(1_000);
        seen.mouse(30_000);
        assert!(!seen.unseen(30_200));
        assert!(seen.unseen(30_800));
        seen.rearmed(30_800);
        assert!(!seen.unseen(30_800));
        assert_eq!(seen.keyboard_tick(), 30_800);
    }
}
