use std::thread::sleep;
use std::time::Duration;

const HOLD_POLL: Duration = Duration::from_millis(16);
const LEAVE_POLLS: usize = 60;
const HOLD_POLLS: usize = 25;
const HOLD_REAPPLY_LIMIT: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HoldOutcome {
    Kept,
    Reapplied(u8),
    Lost,
    NeverLeft,
}

impl HoldOutcome {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Kept => "kept",
            Self::Reapplied(_) => "reapplied",
            Self::Lost => "lost",
            Self::NeverLeft => "never-left",
        }
    }
}

pub(super) fn hold_gone(gone: impl FnMut() -> bool, reapply: impl FnMut() -> bool) -> HoldOutcome {
    hold_gone_with(gone, reapply, || sleep(HOLD_POLL), LEAVE_POLLS, HOLD_POLLS)
}

fn hold_gone_with(
    mut gone: impl FnMut() -> bool,
    mut reapply: impl FnMut() -> bool,
    mut pause: impl FnMut(),
    leave_polls: usize,
    hold_polls: usize,
) -> HoldOutcome {
    let mut seen_gone = false;
    let mut reapplied = 0;
    let mut polls_left = leave_polls;
    while polls_left > 0 {
        polls_left -= 1;
        if gone() {
            if !seen_gone {
                seen_gone = true;
                polls_left = hold_polls;
            }
        } else if seen_gone {
            if reapplied >= HOLD_REAPPLY_LIMIT {
                return HoldOutcome::Lost;
            }
            reapplied += 1;
            let _ = reapply();
            polls_left = hold_polls;
        }
        pause();
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

    use super::{hold_gone_with, HoldOutcome, HOLD_REAPPLY_LIMIT};

    fn run(states: &[bool]) -> (HoldOutcome, usize) {
        let polls = states.len();
        let states = RefCell::new(states.iter().copied());
        let reapplies = Cell::new(0);
        let outcome = hold_gone_with(
            || states.borrow_mut().next().unwrap_or(true),
            || {
                reapplies.set(reapplies.get() + 1);
                true
            },
            || {},
            polls,
            polls,
        );
        (outcome, reapplies.get())
    }

    #[test]
    fn a_window_that_stays_gone_is_left_alone() {
        assert_eq!(run(&[true; 8]), (HoldOutcome::Kept, 0));
    }

    #[test]
    fn a_late_frame_that_brings_the_window_back_is_minimized_again() {
        assert_eq!(
            run(&[true, true, false, true, true]),
            (HoldOutcome::Reapplied(1), 1)
        );
    }

    #[test]
    fn the_window_is_not_reapplied_before_it_first_leaves_the_screen() {
        assert_eq!(run(&[false, false, true, true]), (HoldOutcome::Kept, 0));
        assert_eq!(run(&[false; 4]), (HoldOutcome::NeverLeft, 0));
    }

    #[test]
    fn an_app_that_keeps_coming_back_is_given_up_after_the_limit() {
        let mut states = vec![true];
        states.extend(std::iter::repeat_n(false, 8));
        assert_eq!(
            run(&states),
            (HoldOutcome::Lost, usize::from(HOLD_REAPPLY_LIMIT))
        );
    }
}
