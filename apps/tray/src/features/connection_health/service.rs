use std::time::{Duration, Instant};

use qol_connection_health::{ConnectionHealth, Level, Tracker};
use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};

const STEADY_PROBE: Duration = Duration::from_secs(60);
const UNSETTLED_PROBE: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Step {
    pub(super) publish: Option<ConnectionHealth>,
    pub(super) probe: bool,
    pub(super) deadline: Option<Instant>,
}

pub(super) async fn run() {
    let shared = super::shared();
    let mut published = ConnectionHealth::default();
    let mut last_probe: Option<Instant> = None;
    loop {
        let now = Instant::now();
        let interested = super::interested(now);
        let step = step(&shared.tracker(), now, published, last_probe, interested);
        if let Some(health) = step.publish {
            publish(health);
            published = health;
        }
        if step.probe {
            last_probe = Some(now);
            tokio::spawn(super::probe::run(super::Watch::begin(shared)));
            continue;
        }
        let wake = shared.wake.notified();
        match step.deadline {
            Some(deadline) => {
                tokio::select! {
                    _ = wake => {}
                    _ = tokio::time::sleep_until(deadline.into()) => {}
                }
            }
            None => wake.await,
        }
    }
}

pub(super) fn step(
    tracker: &Tracker,
    now: Instant,
    published: ConnectionHealth,
    last_probe: Option<Instant>,
    interested: bool,
) -> Step {
    let health = tracker.health(now);
    let latest = tracker.last_observed().max(last_probe);
    let probe_at =
        next_probe(now, health.level, latest, tracker.waiting_since()).filter(|_| interested);
    Step {
        publish: (!health.same_signal(&published)).then_some(health),
        probe: probe_at.is_some_and(|at| at <= now),
        deadline: tracker.next_change(now).into_iter().chain(probe_at).min(),
    }
}

pub(super) fn next_probe(
    now: Instant,
    level: Level,
    latest: Option<Instant>,
    waiting_since: Option<Instant>,
) -> Option<Instant> {
    if waiting_since.is_some_and(|since| now < since + super::THRESHOLDS.bad) {
        return None;
    }
    let Some(last) = latest else {
        return Some(now);
    };
    let every = match level {
        Level::Good => STEADY_PROBE,
        Level::Unknown | Level::Slow | Level::Bad => UNSETTLED_PROBE,
    };
    Some(last + every)
}

fn publish(health: ConnectionHealth) {
    log::debug!(
        "connection health is {:?} ({:?}, {:?} ms)",
        health.level,
        health.cause,
        health.latency_ms
    );
    if crate::runtime::is_subscribed(RuntimeEventKind::ConnectionHealthChanged) {
        crate::runtime::publish(&[RuntimeEvent::ConnectionHealthChanged { health }]);
    }
}
