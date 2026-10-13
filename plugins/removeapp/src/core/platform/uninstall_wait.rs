use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Limits {
    pub(super) overall: Duration,
    pub(super) unregister: Duration,
    pub(super) settle: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Observation {
    pub(super) elapsed: Duration,
    pub(super) uninstaller_running: bool,
    pub(super) handler_running: bool,
    pub(super) registered: bool,
    pub(super) exited_for: Option<Duration>,
    pub(super) unregistered_for: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Wait,
    Finished,
    StillInstalled,
    TimedOut,
}

pub(super) fn step(seen: &Observation, limits: &Limits) -> Step {
    if !seen.registered {
        let settled = seen
            .unregistered_for
            .is_some_and(|unregistered| unregistered >= limits.settle);
        if !seen.uninstaller_running || settled {
            return Step::Finished;
        }
        return Step::Wait;
    }
    if seen.elapsed >= limits.overall {
        return Step::TimedOut;
    }
    match seen.exited_for {
        Some(exited) if !seen.handler_running && exited >= limits.unregister => {
            Step::StillInstalled
        }
        _ => Step::Wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits {
        overall: Duration::from_secs(600),
        unregister: Duration::from_secs(15),
        settle: Duration::from_secs(5),
    };

    fn seconds(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    fn running(elapsed: u64) -> Observation {
        Observation {
            elapsed: seconds(elapsed),
            uninstaller_running: true,
            handler_running: false,
            registered: true,
            exited_for: None,
            unregistered_for: None,
        }
    }

    fn exited(elapsed: u64, exited_for: u64) -> Observation {
        Observation {
            uninstaller_running: false,
            exited_for: Some(seconds(exited_for)),
            ..running(elapsed)
        }
    }

    #[test]
    fn a_running_uninstaller_is_waited_for_until_the_overall_limit() {
        assert_eq!(step(&running(1), &LIMITS), Step::Wait);
        assert_eq!(step(&running(599), &LIMITS), Step::Wait);
        assert_eq!(step(&running(600), &LIMITS), Step::TimedOut);
    }

    #[test]
    fn an_exited_uninstaller_finishes_once_the_app_is_unregistered() {
        let unregistered = Observation {
            registered: false,
            unregistered_for: Some(Duration::ZERO),
            ..exited(30, 0)
        };
        assert_eq!(step(&unregistered, &LIMITS), Step::Finished);
    }

    #[test]
    fn an_exited_uninstaller_without_a_hand_off_gets_the_unregister_window() {
        assert_eq!(step(&exited(20, 14), &LIMITS), Step::Wait);
        assert_eq!(step(&exited(21, 15), &LIMITS), Step::StillInstalled);
    }

    #[test]
    fn a_hand_off_keeps_waiting_while_its_handler_runs() {
        let handed_off = |elapsed, exited_for| Observation {
            handler_running: true,
            ..exited(elapsed, exited_for)
        };
        assert_eq!(step(&handed_off(120, 119), &LIMITS), Step::Wait);
        assert_eq!(step(&handed_off(600, 599), &LIMITS), Step::TimedOut);
        let unregistered = Observation {
            registered: false,
            unregistered_for: Some(Duration::ZERO),
            ..handed_off(240, 239)
        };
        assert_eq!(step(&unregistered, &LIMITS), Step::Finished);
    }

    #[test]
    fn a_long_lived_uninstaller_finishes_after_the_app_stays_unregistered() {
        let unregistered = |for_seconds| Observation {
            registered: false,
            unregistered_for: Some(seconds(for_seconds)),
            ..running(60)
        };
        assert_eq!(step(&unregistered(4), &LIMITS), Step::Wait);
        assert_eq!(step(&unregistered(5), &LIMITS), Step::Finished);
    }
}
