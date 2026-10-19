use std::time::{Duration, Instant};

use qol_connection_health::{ConnectionHealth, Level};
use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};

const STEADY_PROBE: Duration = Duration::from_secs(60);
const UNSETTLED_PROBE: Duration = Duration::from_secs(10);

pub(super) async fn run() {
    let mut published = ConnectionHealth::default();
    let mut last_probe: Option<Instant> = None;
    loop {
        let now = Instant::now();
        let (health, next_change, probe_at) = {
            let tracker = super::tracker();
            let health = tracker.health(now);
            let latest = tracker.last_observed().max(last_probe);
            let probe_at = next_probe(now, health.level, latest, tracker.waiting_since());
            (health, tracker.next_change(now), probe_at)
        };
        if !health.same_signal(&published) {
            publish(health);
            published = health;
        }
        let probe_at = probe_at.filter(|_| super::interested(now));
        if probe_at.is_some_and(|at| at <= now) {
            last_probe = Some(now);
            tokio::spawn(super::probe::run(super::Watch::begin()));
            continue;
        }
        let wake = super::shared().wake.notified();
        match next_change.into_iter().chain(probe_at).min() {
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
