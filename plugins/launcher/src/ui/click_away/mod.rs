mod platform;

pub(crate) use platform::{start, Monitor};

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
    use super::ArmState;

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
