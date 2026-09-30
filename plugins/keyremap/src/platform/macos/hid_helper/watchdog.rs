use std::time::{Duration, Instant};

pub(crate) const SILENCE_LIMIT: Duration = Duration::from_millis(200);
pub(crate) const SEIZE_RETRY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Seize,
    Release,
    Hold,
}

#[derive(Debug, Default)]
pub(crate) struct Watchdog {
    session: Option<u64>,
    last_heartbeat: Option<Instant>,
    keyboard_ready: bool,
    input_monitoring: bool,
    seized: bool,
    devices_changed: bool,
    retry_at: Option<Instant>,
}

impl Watchdog {
    pub(crate) fn session_opened(&mut self, generation: u64) {
        self.session = Some(generation);
        self.last_heartbeat = None;
    }

    pub(crate) fn session_closed(&mut self, generation: u64) {
        if self.is_current(generation) {
            self.session = None;
            self.last_heartbeat = None;
        }
    }

    pub(crate) fn heartbeat(&mut self, generation: u64, now: Instant) {
        if self.is_current(generation) {
            self.last_heartbeat = Some(now);
        }
    }

    pub(crate) fn is_current(&self, generation: u64) -> bool {
        self.session == Some(generation)
    }

    pub(crate) fn accepts_emit(&self, generation: u64) -> bool {
        self.is_current(generation) && self.seized
    }

    pub(crate) fn set_keyboard_ready(&mut self, ready: bool) {
        self.keyboard_ready = ready;
    }

    pub(crate) fn set_input_monitoring(&mut self, granted: bool) {
        self.input_monitoring = granted;
    }

    pub(crate) fn device_arrived(&mut self) {
        self.devices_changed = true;
        self.retry_at = None;
    }

    pub(crate) fn seize_failed(&mut self, now: Instant) {
        self.seized = false;
        self.retry_at = Some(now + SEIZE_RETRY);
    }

    pub(crate) fn tick(&mut self, now: Instant) -> Action {
        let heard_recently = self
            .last_heartbeat
            .is_some_and(|at| now.saturating_duration_since(at) <= SILENCE_LIMIT);
        let retry_due = self.retry_at.is_none_or(|at| now >= at);
        let wanted = self.keyboard_ready && self.input_monitoring && heard_recently;
        let action = match (self.seized, wanted) {
            (false, true) if retry_due => Action::Seize,
            (false, true) => Action::Hold,
            (true, false) => Action::Release,
            (true, true) if self.devices_changed => Action::Seize,
            _ => Action::Hold,
        };
        self.devices_changed = false;
        self.seized = match action {
            Action::Seize => {
                self.retry_at = None;
                true
            }
            Action::Release => false,
            Action::Hold => self.seized,
        };
        action
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn ready() -> Watchdog {
        let mut watchdog = Watchdog::default();
        watchdog.set_keyboard_ready(true);
        watchdog.set_input_monitoring(true);
        watchdog
    }

    #[test]
    fn seizes_only_after_a_heartbeat_with_the_keyboard_ready() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        assert_eq!(watchdog.tick(start), Action::Hold);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        assert_eq!(
            watchdog.tick(start + Duration::from_millis(25)),
            Action::Hold
        );
    }

    #[test]
    fn a_failed_seize_is_retried_after_a_second_or_when_a_keyboard_arrives() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        watchdog.seize_failed(start);
        assert!(!watchdog.accepts_emit(1));

        let soon = start + Duration::from_millis(100);
        watchdog.heartbeat(1, soon);
        assert_eq!(watchdog.tick(soon), Action::Hold);

        let later = start + SEIZE_RETRY;
        watchdog.heartbeat(1, later);
        assert_eq!(watchdog.tick(later), Action::Seize);

        watchdog.seize_failed(later);
        watchdog.device_arrived();
        assert_eq!(watchdog.tick(later), Action::Seize);
    }

    #[test]
    fn emits_are_refused_once_the_keyboards_are_released() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        assert!(!watchdog.accepts_emit(1));
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        assert!(watchdog.accepts_emit(1));
        assert!(!watchdog.accepts_emit(2));
        assert_eq!(
            watchdog.tick(start + SILENCE_LIMIT + Duration::from_millis(1)),
            Action::Release
        );
        assert!(!watchdog.accepts_emit(1));
    }

    #[test]
    fn releases_after_200ms_of_silence_and_reseizes_on_the_next_heartbeat() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        assert_eq!(watchdog.tick(start + SILENCE_LIMIT), Action::Hold);
        assert_eq!(
            watchdog.tick(start + SILENCE_LIMIT + Duration::from_millis(1)),
            Action::Release
        );
        let later = start + Duration::from_secs(1);
        watchdog.heartbeat(1, later);
        assert_eq!(watchdog.tick(later), Action::Seize);
    }

    #[test]
    fn releases_at_once_when_the_virtual_keyboard_goes_away() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        watchdog.set_keyboard_ready(false);
        assert_eq!(watchdog.tick(start), Action::Release);
    }

    #[test]
    fn releases_when_the_session_closes() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        watchdog.session_closed(1);
        assert_eq!(watchdog.tick(start), Action::Release);
    }

    #[test]
    fn no_input_monitoring_never_seizes() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.set_input_monitoring(false);
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Hold);
    }

    #[test]
    fn a_stale_session_cannot_keep_or_drop_the_seize() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);

        watchdog.session_opened(2);
        watchdog.heartbeat(1, start);
        assert!(!watchdog.is_current(1));
        assert_eq!(watchdog.tick(start), Action::Release);

        watchdog.heartbeat(2, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        watchdog.session_closed(1);
        assert_eq!(watchdog.tick(start), Action::Hold);
    }

    #[test]
    fn device_arrival_while_seized_asks_for_a_seize() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        watchdog.device_arrived();
        assert_eq!(watchdog.tick(start), Action::Seize);
        assert_eq!(watchdog.tick(start), Action::Hold);
    }

    #[test]
    fn device_arrival_while_released_does_nothing() {
        let mut watchdog = ready();
        watchdog.device_arrived();
        assert_eq!(watchdog.tick(Instant::now()), Action::Hold);
    }
}
