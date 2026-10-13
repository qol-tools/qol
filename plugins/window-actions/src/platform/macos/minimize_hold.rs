use std::thread::sleep;
use std::time::{Duration, Instant};

use super::objc::CGEventSourceSecondsSinceLastEventType;

const HOLD_POLL: Duration = Duration::from_millis(16);
const LEAVE_POLLS: usize = 6;
const HOLD_POLLS: usize = 25;
const HOLD_REAPPLY_LIMIT: u8 = 3;
const REAPPLY_SETTLE_POLLS: usize = 3;
const HID_SYSTEM_STATE: i32 = 1;
const LEFT_MOUSE_DOWN: u32 = 1;
const RIGHT_MOUSE_DOWN: u32 = 3;
const KEY_DOWN: u32 = 10;
const OTHER_MOUSE_DOWN: u32 = 25;
const PRESS_EVENT_TYPES: [u32; 4] = [
    LEFT_MOUSE_DOWN,
    RIGHT_MOUSE_DOWN,
    KEY_DOWN,
    OTHER_MOUSE_DOWN,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HoldOutcome {
    Kept,
    Reapplied(u8),
    Yielded,
    Lost,
    NeverLeft,
}

impl HoldOutcome {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Kept => "kept",
            Self::Reapplied(_) => "reapplied",
            Self::Yielded => "yielded",
            Self::Lost => "lost",
            Self::NeverLeft => "never-left",
        }
    }
}

struct Probes<Gone, Reapply, Acted, Pause> {
    gone: Gone,
    reapply: Reapply,
    user_acted: Acted,
    pause: Pause,
}

pub(super) fn hold_gone(gone: impl FnMut() -> bool, reapply: impl FnMut() -> bool) -> HoldOutcome {
    let started = Instant::now();
    hold_gone_with(
        Probes {
            gone,
            reapply,
            user_acted: || user_pressed_since(started),
            pause: || sleep(HOLD_POLL),
        },
        LEAVE_POLLS,
        HOLD_POLLS,
    )
}

fn user_pressed_since(started: Instant) -> bool {
    let elapsed = started.elapsed().as_secs_f64();
    PRESS_EVENT_TYPES.iter().any(|&event_type| {
        let since = unsafe { CGEventSourceSecondsSinceLastEventType(HID_SYSTEM_STATE, event_type) };
        since < elapsed
    })
}

fn hold_gone_with(
    mut probes: Probes<
        impl FnMut() -> bool,
        impl FnMut() -> bool,
        impl FnMut() -> bool,
        impl FnMut(),
    >,
    leave_polls: usize,
    hold_polls: usize,
) -> HoldOutcome {
    let mut seen_gone = false;
    let mut reapplied = 0;
    let mut settle_polls = 0;
    let mut polls_left = leave_polls;
    while polls_left > 0 {
        polls_left -= 1;
        if (probes.gone)() {
            settle_polls = 0;
            if !seen_gone {
                seen_gone = true;
                polls_left = hold_polls;
            }
        } else if seen_gone {
            if (probes.user_acted)() {
                return HoldOutcome::Yielded;
            }
            if settle_polls > 0 {
                settle_polls -= 1;
            } else {
                if reapplied >= HOLD_REAPPLY_LIMIT {
                    return HoldOutcome::Lost;
                }
                reapplied += 1;
                settle_polls = REAPPLY_SETTLE_POLLS;
                polls_left = polls_left.max(REAPPLY_SETTLE_POLLS + 1);
                let _ = (probes.reapply)();
            }
        }
        (probes.pause)();
    }
    match (seen_gone, reapplied) {
        (false, _) => HoldOutcome::NeverLeft,
        (true, 0) => HoldOutcome::Kept,
        (true, count) => HoldOutcome::Reapplied(count),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::{hold_gone_with, HoldOutcome, Probes, HOLD_REAPPLY_LIMIT};

    fn run(states: &[bool], user_acted: bool) -> (HoldOutcome, usize) {
        let polls = states.len();
        let states = RefCell::new(states.iter().copied());
        let reapplies = Cell::new(0);
        let outcome = hold_gone_with(
            Probes {
                gone: || states.borrow_mut().next().unwrap_or(true),
                reapply: || {
                    reapplies.set(reapplies.get() + 1);
                    true
                },
                user_acted: || user_acted,
                pause: || {},
            },
            polls,
            polls,
        );
        (outcome, reapplies.get())
    }

    #[test]
    fn a_window_that_stays_gone_is_left_alone() {
        assert_eq!(run(&[true; 8], false), (HoldOutcome::Kept, 0));
    }

    #[test]
    fn a_late_frame_that_brings_the_window_back_is_minimized_again() {
        assert_eq!(
            run(&[true, true, false, true, true], false),
            (HoldOutcome::Reapplied(1), 1)
        );
    }

    #[test]
    fn every_separate_return_is_minimized_again_at_once() {
        let returns = Cell::new(0);
        let polls = Cell::new(0);
        let outcome = hold_gone_with(
            Probes {
                gone: || {
                    polls.set(polls.get() + 1);
                    !matches!(polls.get(), 3 | 6)
                },
                reapply: || {
                    returns.set(returns.get() + 1);
                    true
                },
                user_acted: || false,
                pause: || {},
            },
            7,
            7,
        );
        assert_eq!((outcome, returns.get()), (HoldOutcome::Reapplied(2), 2));
    }

    #[test]
    fn a_window_the_user_brings_back_stays_back() {
        assert_eq!(
            run(&[true, true, false, false, false], true),
            (HoldOutcome::Yielded, 0)
        );
    }

    #[test]
    fn the_window_is_not_reapplied_before_it_first_leaves_the_screen() {
        assert_eq!(
            run(&[false, false, true, true], false),
            (HoldOutcome::Kept, 0)
        );
        assert_eq!(run(&[false; 4], false), (HoldOutcome::NeverLeft, 0));
    }

    #[test]
    fn an_app_that_keeps_coming_back_is_given_up_after_the_limit() {
        let mut states = vec![true];
        states.extend(std::iter::repeat_n(false, 20));
        assert_eq!(
            run(&states, false),
            (HoldOutcome::Lost, usize::from(HOLD_REAPPLY_LIMIT))
        );
    }
}
