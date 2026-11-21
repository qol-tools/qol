mod quiet;
mod state;

use qol_sync::{Condvar, Mutex};
use std::sync::Once;
use std::time::Duration;

pub use state::{Health, Mechanism, Outcome, Permission, TakeoverState};

const STARTING_RECHECK: Duration = Duration::from_secs(1);
const RECHECK: Duration = Duration::from_secs(30);

static STATE: Mutex<Option<TakeoverState>> = Mutex::new(None);
static CHANGES: Mutex<u64> = Mutex::new(0);
static CHANGED: Condvar = Condvar::new();
static STARTED: Once = Once::new();

pub fn start() {
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new()
            .name("qol-notification-takeover".to_string())
            .spawn(run);
        if let Err(error) = spawned {
            log::warn!("notification takeover: not started: {error}");
        }
    });
}

pub fn started() -> bool {
    STARTED.is_completed()
}

pub fn poke() {
    *CHANGES.lock() += 1;
    CHANGED.notify_all();
}

pub fn state() -> TakeoverState {
    STATE.lock().clone().unwrap_or_else(TakeoverState::off)
}

pub fn holds_banner_switch() -> bool {
    state().mechanisms.iter().any(|mechanism| {
        mechanism.name == quiet::BANNER_SWITCH && mechanism.health == Health::Active
    })
}

fn run() {
    let mut quiet = quiet::Quiet::default();
    loop {
        let changes = *CHANGES.lock();
        let wanted = super::takeover_wanted();
        let mut mechanisms = super::mirror::apply(wanted);
        mechanisms.extend(quiet.apply(wanted));
        let next = TakeoverState::new(wanted, super::enabled(), mechanisms);
        let recheck = recheck_after(next.value);
        publish(next);
        wait_for_change(changes, recheck);
    }
}

fn publish(next: TakeoverState) {
    let mut state = STATE.lock();
    if state.as_ref() == Some(&next) {
        return;
    }
    log::info!("notification takeover: {}", next.describe());
    *state = Some(next);
}

fn recheck_after(outcome: Outcome) -> Option<Duration> {
    match outcome {
        Outcome::Off | Outcome::Unsupported => None,
        Outcome::Starting => Some(STARTING_RECHECK),
        Outcome::Toasts
        | Outcome::ToastsPartly
        | Outcome::ToastsAndBanners
        | Outcome::Hidden
        | Outcome::Muted
        | Outcome::Blocked
        | Outcome::Failed => Some(RECHECK),
    }
}

fn wait_for_change(seen: u64, recheck: Option<Duration>) {
    let mut changes = CHANGES.lock();
    match recheck {
        Some(limit) if *changes == seen => drop(CHANGED.wait_timeout(changes, limit)),
        Some(_) => {}
        None => {
            while *changes == seen {
                changes = CHANGED.wait(changes);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_working_or_broken_takeover_is_checked_again() {
        assert_eq!(recheck_after(Outcome::Off), None);
        assert_eq!(recheck_after(Outcome::Unsupported), None);
        assert_eq!(recheck_after(Outcome::Starting), Some(STARTING_RECHECK));
        for outcome in [Outcome::Toasts, Outcome::Blocked, Outcome::Hidden] {
            assert_eq!(recheck_after(outcome), Some(RECHECK), "{outcome:?}");
        }
    }
}
