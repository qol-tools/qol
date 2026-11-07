use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

pub const PAIRING_WINDOW: Duration = Duration::from_secs(60);

pub type SharedPairing = Arc<Mutex<PairingSession>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingPhase {
    NoDongle,
    Searching(Duration),
    Fresh,
    Ended,
}

#[derive(Debug, Default, Clone)]
pub struct PairingSession {
    started: Option<Instant>,
    ended: Option<Instant>,
    joined: Vec<String>,
}

impl PairingSession {
    pub fn start(&mut self, now: Instant) {
        self.started = Some(now);
        self.ended = None;
        self.joined.clear();
    }

    pub fn stop(&mut self, now: Instant) {
        if self.started.is_some() && self.ended.is_none() {
            self.ended = Some(now);
        }
    }

    pub fn light_joined(&mut self, ieee: String, known: bool, now: Instant) -> bool {
        if known || !matches!(self.phase(true, now), PairingPhase::Searching(_)) {
            return false;
        }
        if !self.joined.contains(&ieee) {
            self.joined.push(ieee);
        }
        self.stop(now);
        true
    }

    pub fn joined(&self) -> &[String] {
        &self.joined
    }

    pub fn phase(&self, dongle_ready: bool, now: Instant) -> PairingPhase {
        if !dongle_ready {
            return PairingPhase::NoDongle;
        }
        let Some(started) = self.started else {
            return PairingPhase::Fresh;
        };
        if self.ended.is_some() {
            return PairingPhase::Ended;
        }
        let elapsed = now.saturating_duration_since(started);
        if elapsed >= PAIRING_WINDOW {
            return PairingPhase::Ended;
        }
        PairingPhase::Searching(PAIRING_WINDOW - elapsed)
    }
}

pub fn snapshot(pairing: &SharedPairing) -> PairingSession {
    pairing
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub fn update(pairing: &SharedPairing, change: impl FnOnce(&mut PairingSession)) {
    change(&mut pairing.lock().unwrap_or_else(PoisonError::into_inner));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_follows_the_pairing_window() {
        let start = Instant::now();
        let mut session = PairingSession::default();
        assert_eq!(session.phase(false, start), PairingPhase::NoDongle);
        assert_eq!(session.phase(true, start), PairingPhase::Fresh);

        session.start(start);
        let cases = [
            (Duration::ZERO, PairingPhase::Searching(PAIRING_WINDOW)),
            (
                Duration::from_secs(45),
                PairingPhase::Searching(Duration::from_secs(15)),
            ),
            (PAIRING_WINDOW, PairingPhase::Ended),
            (Duration::from_secs(600), PairingPhase::Ended),
        ];
        for (elapsed, expected) in cases {
            assert_eq!(
                session.phase(true, start + elapsed),
                expected,
                "{elapsed:?}"
            );
        }
        assert_eq!(session.phase(false, start), PairingPhase::NoDongle);
    }

    #[test]
    fn a_new_light_ends_the_search_and_a_new_search_forgets_it() {
        let start = Instant::now();
        let mut session = PairingSession::default();
        session.start(start);

        assert!(session.light_joined("AA".into(), false, start));
        assert!(!session.light_joined("AA".into(), false, start));
        assert_eq!(session.joined(), ["AA".to_string()]);
        assert_eq!(session.phase(true, start), PairingPhase::Ended);

        session.start(start);
        assert!(session.joined().is_empty());
        assert_eq!(
            session.phase(true, start),
            PairingPhase::Searching(PAIRING_WINDOW)
        );
    }

    #[test]
    fn a_rejoin_outside_a_search_or_of_a_known_light_is_not_counted() {
        let start = Instant::now();
        let cases = [
            ("no search yet", None, false),
            ("known light mid search", Some(Duration::from_secs(5)), true),
            ("window over", Some(PAIRING_WINDOW), false),
        ];
        for (case, started_ago, known) in cases {
            let now = start + Duration::from_secs(120);
            let mut session = PairingSession::default();
            if let Some(ago) = started_ago {
                session.start(now - ago);
            }
            let before = session.phase(true, now);

            assert!(!session.light_joined("AA".into(), known, now), "{case}");
            assert!(session.joined().is_empty(), "{case}");
            assert_eq!(session.phase(true, now), before, "{case}");
        }
    }

    #[test]
    fn stop_only_ends_a_running_search() {
        let start = Instant::now();
        let mut session = PairingSession::default();
        session.stop(start);
        assert_eq!(session.phase(true, start), PairingPhase::Fresh);

        session.start(start);
        session.stop(start + Duration::from_secs(5));
        assert_eq!(
            session.phase(true, start + Duration::from_secs(6)),
            PairingPhase::Ended
        );
    }
}
