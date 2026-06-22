mod platform;

pub(crate) use platform::{start, Monitor};

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub(crate) struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub(crate) struct ScreenFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[allow(dead_code)]
fn contains_point(frame: ScreenFrame, point: ScreenPoint) -> bool {
    point.x >= frame.x
        && point.x <= frame.x + frame.width
        && point.y >= frame.y
        && point.y <= frame.y + frame.height
}

#[allow(dead_code)]
pub(crate) fn click_is_outside(frame: ScreenFrame, point: ScreenPoint) -> bool {
    !contains_point(frame, point)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ArmState {
    #[default]
    Idle,
    Armed,
    Unsupported,
}

impl ArmState {
    pub(crate) fn should_start(self, is_showing: bool) -> bool {
        matches!(self, ArmState::Idle) && is_showing
    }

    pub(crate) fn started(self, ok: bool) -> ArmState {
        if ok {
            ArmState::Armed
        } else {
            ArmState::Unsupported
        }
    }

    pub(crate) fn stopped(self) -> ArmState {
        match self {
            ArmState::Armed => ArmState::Idle,
            ArmState::Idle => ArmState::Idle,
            ArmState::Unsupported => ArmState::Unsupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{click_is_outside, ArmState, ScreenFrame, ScreenPoint};

    fn sample_frame() -> ScreenFrame {
        ScreenFrame {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        }
    }

    #[test]
    fn clicks_inside_the_frame_are_not_outside() {
        let cases = [(10.0, 20.0), (60.0, 45.0), (110.0, 70.0)];
        for (x, y) in cases {
            assert!(
                !click_is_outside(sample_frame(), ScreenPoint { x, y }),
                "expected inside for ({x}, {y})"
            );
        }
    }

    #[test]
    fn clicks_outside_the_frame_are_outside() {
        let cases = [(9.9, 30.0), (200.0, 30.0), (30.0, 19.9), (30.0, 70.1)];
        for (x, y) in cases {
            assert!(
                click_is_outside(sample_frame(), ScreenPoint { x, y }),
                "expected outside for ({x}, {y})"
            );
        }
    }

    #[test]
    fn arm_starts_only_when_idle_and_showing() {
        let cases = [
            (ArmState::Idle, true, true),
            (ArmState::Idle, false, false),
            (ArmState::Armed, true, false),
            (ArmState::Unsupported, true, false),
        ];
        for (state, showing, expected) in cases {
            assert_eq!(
                state.should_start(showing),
                expected,
                "state={state:?} showing={showing}"
            );
        }
    }

    #[test]
    fn failed_start_is_recorded_and_never_retried() {
        let after_failure = ArmState::Idle.started(false);
        assert_eq!(after_failure, ArmState::Unsupported);
        assert!(!after_failure.should_start(true));
        assert_eq!(after_failure.stopped(), ArmState::Unsupported);
        assert!(!after_failure.stopped().should_start(true));
    }

    #[test]
    fn stopping_an_armed_monitor_allows_re_arming() {
        let armed = ArmState::Idle.started(true);
        assert_eq!(armed, ArmState::Armed);
        assert_eq!(armed.stopped(), ArmState::Idle);
        assert!(armed.stopped().should_start(true));
    }
}
