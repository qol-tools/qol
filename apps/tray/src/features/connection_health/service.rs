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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Probes {
    pub(super) last: Option<Instant>,
    pub(super) running: bool,
    pub(super) failures: u32,
}

pub(super) async fn run() {
    let shared = super::shared();
    loop {
        let now = Instant::now();
        let published = shared.published();
        let probes = shared.probes();
        let interested = shared.watching(now)
            || crate::runtime::is_subscribed(RuntimeEventKind::ConnectionHealthChanged);
        let step = step(&shared.tracker(), now, published, probes, interested);
        if let Some(health) = step.publish {
            shared.announce(health, publish);
        }
        if step.probe {
            tokio::spawn(super::probe::run(super::probe::start(shared, now)));
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
    probes: Probes,
    interested: bool,
) -> Step {
    let health = tracker.health(now);
    let probe_at = next_probe(
        now,
        health.level,
        tracker.last_observed(),
        probes,
        tracker.waiting_since(),
    )
    .filter(|_| interested);
    Step {
        publish: (!health.same_signal(&published)).then_some(health),
        probe: probe_at.is_some_and(|at| at <= now),
        deadline: tracker.next_change(now).into_iter().chain(probe_at).min(),
    }
}

pub(super) fn next_probe(
    now: Instant,
    level: Level,
    observed: Option<Instant>,
    probes: Probes,
    waiting_since: Option<Instant>,
) -> Option<Instant> {
    if probes.running {
        return None;
    }
    if let Some(since) = waiting_since {
        let stalled = since + super::THRESHOLDS.bad;
        return Some(
            probes
                .last
                .map_or(stalled, |last| stalled.max(last + UNSETTLED_PROBE)),
        );
    }
    let Some(latest) = observed.max(probes.last) else {
        return Some(now);
    };
    Some(latest + interval(level, probes.failures))
}

fn interval(level: Level, failures: u32) -> Duration {
    match level {
        Level::Good => STEADY_PROBE,
        Level::Unknown | Level::Slow | Level::Bad => UNSETTLED_PROBE
            .saturating_mul(1 << failures.min(3))
            .min(STEADY_PROBE),
    }
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
