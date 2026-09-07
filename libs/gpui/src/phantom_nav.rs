use std::time::{Duration, Instant};

pub const PHANTOM_NAV_WINDOW: Duration = Duration::from_millis(60);

pub const DUPLICATE_NAV_WINDOW: Duration = Duration::from_millis(40);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavAxis {
    Horizontal,
    Vertical,
}

pub fn is_phantom_reversal(
    last_at: Option<Instant>,
    last_direction: Option<f64>,
    direction: f64,
) -> bool {
    let (Some(last), Some(prev)) = (last_at, last_direction) else {
        return false;
    };
    prev != direction && Instant::now().duration_since(last) < PHANTOM_NAV_WINDOW
}

pub fn is_duplicate_step(
    last_at: Option<Instant>,
    last_direction: Option<f64>,
    direction: f64,
) -> bool {
    let (Some(last), Some(prev)) = (last_at, last_direction) else {
        return false;
    };
    prev == direction && Instant::now().duration_since(last) < DUPLICATE_NAV_WINDOW
}

#[derive(Default)]
pub struct PhantomNavGuard {
    horizontal: AxisNav,
    vertical: AxisNav,
}

#[derive(Default)]
struct AxisNav {
    last_direction: Option<f64>,
    last_at: Option<Instant>,
}

impl PhantomNavGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn swallow(&mut self, axis: NavAxis, direction: f64) -> bool {
        let state = match axis {
            NavAxis::Horizontal => &mut self.horizontal,
            NavAxis::Vertical => &mut self.vertical,
        };
        let phantom = is_phantom_reversal(state.last_at, state.last_direction, direction);
        if phantom {
            return true;
        }
        if is_duplicate_step(state.last_at, state.last_direction, direction) {
            return true;
        }
        state.last_direction = Some(direction);
        state.last_at = Some(Instant::now());
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_reversal_is_ignored_as_phantom() {
        let mut guard = PhantomNavGuard::new();

        assert!(!guard.swallow(NavAxis::Vertical, 1.0));
        assert!(
            guard.swallow(NavAxis::Vertical, -1.0),
            "an opposite key within the phantom window must be swallowed"
        );
    }

    #[test]
    fn a_key_delivered_twice_moves_the_selection_once() {
        let mut guard = PhantomNavGuard::new();

        assert!(!guard.swallow(NavAxis::Vertical, 1.0));
        assert!(
            guard.swallow(NavAxis::Vertical, 1.0),
            "the same key redelivered within a few milliseconds is one press"
        );
    }

    #[test]
    fn a_held_key_still_repeats() {
        let mut guard = PhantomNavGuard::new();

        for _ in 0..4 {
            assert!(!guard.swallow(NavAxis::Vertical, 1.0));
            guard.vertical.last_at = Some(Instant::now() - Duration::from_millis(120));
        }
    }

    #[test]
    fn slow_reversal_is_allowed() {
        let mut guard = PhantomNavGuard::new();

        assert!(!guard.swallow(NavAxis::Vertical, 1.0));
        guard.vertical.last_at = Some(Instant::now() - Duration::from_millis(250));
        assert!(
            !guard.swallow(NavAxis::Vertical, -1.0),
            "a human-paced reversal is real navigation"
        );
    }

    #[test]
    fn axes_are_independent() {
        let mut guard = PhantomNavGuard::new();

        assert!(!guard.swallow(NavAxis::Vertical, 1.0));
        assert!(!guard.swallow(NavAxis::Horizontal, 1.0));
        guard.vertical.last_at = Some(Instant::now() - Duration::from_millis(1));
        guard.horizontal.last_at = Some(Instant::now() - Duration::from_millis(1));
        assert!(
            guard.swallow(NavAxis::Vertical, -1.0),
            "a vertical reversal must not consume the horizontal state"
        );
        assert!(
            guard.swallow(NavAxis::Horizontal, -1.0),
            "a horizontal reversal must not consume the vertical state"
        );
    }

    #[test]
    fn swallowed_keys_do_not_update_the_stored_direction() {
        let mut guard = PhantomNavGuard::new();

        assert!(!guard.swallow(NavAxis::Vertical, 1.0));
        assert!(guard.swallow(NavAxis::Vertical, -1.0));
        guard.vertical.last_at = Some(Instant::now() - Duration::from_millis(120));
        assert!(
            !guard.swallow(NavAxis::Vertical, 1.0),
            "the swallow must not have replaced the recorded direction"
        );
    }
}
