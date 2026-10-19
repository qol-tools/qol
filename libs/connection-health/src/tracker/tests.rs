use std::time::{Duration, Instant};

use super::{Outcome, Thresholds, Tracker};
use crate::{Cause, ConnectionHealth, Level};

const THRESHOLDS: Thresholds = Thresholds {
    slow: Duration::from_millis(1500),
    bad: Duration::from_millis(4000),
    memory: Duration::from_secs(180),
};

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn tracker() -> (Tracker, Instant) {
    (Tracker::new(THRESHOLDS), Instant::now())
}

fn answered_after(tracker: &mut Tracker, start: Instant, latency: Duration) -> Instant {
    let ticket = tracker.begin(start);
    let end = start + latency;
    tracker.finish(ticket, Outcome::Answered, end);
    end
}

#[test]
fn nothing_observed_is_unknown() {
    let (tracker, now) = tracker();
    assert_eq!(tracker.health(now), ConnectionHealth::default());
    assert_eq!(tracker.next_change(now), None);
    assert_eq!(tracker.waiting_since(), None);
}

#[test]
fn a_quick_answer_is_good_and_reports_its_latency() {
    let (mut tracker, start) = tracker();
    let end = answered_after(&mut tracker, start, ms(180));
    assert_eq!(
        tracker.health(end),
        ConnectionHealth {
            level: Level::Good,
            cause: None,
            latency_ms: Some(180),
        }
    );
    assert_eq!(tracker.last_observed(), Some(end));
}

#[test]
fn answer_latency_grades_slow_then_bad() {
    let cases = [
        (ms(1499), Level::Good, None),
        (ms(1500), Level::Slow, Some(Cause::SlowAnswers)),
        (ms(3999), Level::Slow, Some(Cause::SlowAnswers)),
        (ms(4000), Level::Bad, Some(Cause::SlowAnswers)),
    ];
    for (latency, level, cause) in cases {
        let (mut tracker, start) = tracker();
        let end = answered_after(&mut tracker, start, latency);
        let health = tracker.health(end);
        assert_eq!(health.level, level, "latency {latency:?}");
        assert_eq!(health.cause, cause, "latency {latency:?}");
    }
}

#[test]
fn an_unreachable_server_is_bad() {
    let (mut tracker, start) = tracker();
    let ticket = tracker.begin(start);
    tracker.finish(ticket, Outcome::Unreachable, start + ms(300));
    assert_eq!(
        tracker.health(start + ms(300)),
        ConnectionHealth {
            level: Level::Bad,
            cause: Some(Cause::Unreachable),
            latency_ms: None,
        }
    );
}

#[test]
fn one_answer_after_a_failure_recovers() {
    let (mut tracker, start) = tracker();
    let ticket = tracker.begin(start);
    tracker.finish(ticket, Outcome::Unreachable, start + ms(100));
    let end = answered_after(&mut tracker, start + ms(200), ms(150));
    assert_eq!(tracker.health(end).level, Level::Good);
}

#[test]
fn a_request_left_waiting_turns_slow_then_bad() {
    let (mut tracker, start) = tracker();
    let _ticket = tracker.begin(start);
    assert_eq!(tracker.health(start + ms(1499)).level, Level::Unknown);
    let slow = tracker.health(start + ms(1500));
    assert_eq!(
        (slow.level, slow.cause),
        (Level::Slow, Some(Cause::Waiting))
    );
    let bad = tracker.health(start + ms(4000));
    assert_eq!((bad.level, bad.cause), (Level::Bad, Some(Cause::Waiting)));
    assert_eq!(tracker.waiting_since(), Some(start));
}

#[test]
fn waiting_keeps_the_last_answer_latency() {
    let (mut tracker, start) = tracker();
    let end = answered_after(&mut tracker, start, ms(200));
    let _ticket = tracker.begin(end);
    assert_eq!(
        tracker.health(end + ms(2000)),
        ConnectionHealth {
            level: Level::Slow,
            cause: Some(Cause::Waiting),
            latency_ms: Some(200),
        }
    );
}

#[test]
fn a_short_wait_does_not_hide_a_known_failure() {
    let (mut tracker, start) = tracker();
    let failed = tracker.begin(start);
    tracker.finish(failed, Outcome::Unreachable, start + ms(100));
    let _retry = tracker.begin(start + ms(200));
    let health = tracker.health(start + ms(5000));
    assert_eq!(
        (health.level, health.cause),
        (Level::Bad, Some(Cause::Unreachable))
    );
}

#[test]
fn the_oldest_waiting_request_decides() {
    let (mut tracker, start) = tracker();
    let _old = tracker.begin(start);
    let young = tracker.begin(start + ms(3000));
    tracker.abandon(young);
    let _young = tracker.begin(start + ms(3500));
    assert_eq!(tracker.health(start + ms(4000)).level, Level::Bad);
}

#[test]
fn an_abandoned_request_leaves_no_sample() {
    let (mut tracker, start) = tracker();
    let ticket = tracker.begin(start);
    tracker.abandon(ticket);
    assert_eq!(tracker.waiting_since(), None);
    assert_eq!(tracker.last_observed(), None);
    assert_eq!(
        tracker.health(start + ms(9000)),
        ConnectionHealth::default()
    );
}

#[test]
fn answers_are_smoothed() {
    let (mut tracker, start) = tracker();
    let first = answered_after(&mut tracker, start, ms(200));
    let second = answered_after(&mut tracker, first, ms(3000));
    let health = tracker.health(second);
    assert_eq!(health.latency_ms, Some(1600));
    assert_eq!(health.level, Level::Slow);
    let third = answered_after(&mut tracker, second, ms(200));
    assert_eq!(tracker.health(third).latency_ms, Some(900));
    assert_eq!(tracker.health(third).level, Level::Good);
}

#[test]
fn smoothing_restarts_once_the_last_sample_is_forgotten() {
    let (mut tracker, start) = tracker();
    let first = answered_after(&mut tracker, start, ms(3000));
    let later = first + THRESHOLDS.memory;
    let end = answered_after(&mut tracker, later, ms(100));
    assert_eq!(tracker.health(end).latency_ms, Some(100));
}

#[test]
fn health_is_forgotten_after_the_memory_window() {
    let (mut tracker, start) = tracker();
    let end = answered_after(&mut tracker, start, ms(100));
    assert_eq!(
        tracker.health(end + THRESHOLDS.memory - ms(1)).level,
        Level::Good
    );
    assert_eq!(
        tracker.health(end + THRESHOLDS.memory),
        ConnectionHealth::default()
    );
}

#[test]
fn a_newer_answer_clears_a_request_that_stalled() {
    let (mut tracker, start) = tracker();
    let stalled = tracker.begin(start);
    assert_eq!(tracker.health(start + ms(4000)).level, Level::Bad);
    let probe = tracker.begin(start + ms(4000));
    tracker.finish(probe, Outcome::Answered, start + ms(4100));
    assert_eq!(
        tracker.health(start + ms(60_000)),
        ConnectionHealth {
            level: Level::Good,
            cause: None,
            latency_ms: Some(100),
        }
    );
    assert_eq!(tracker.waiting_since(), None);
    assert_eq!(
        tracker.next_change(start + ms(4100)),
        Some(start + ms(4100) + THRESHOLDS.memory)
    );
    tracker.finish(stalled, Outcome::Answered, start + ms(90_000));
    assert_eq!(tracker.health(start + ms(90_000)).latency_ms, Some(100));
}

#[test]
fn an_older_request_failing_late_does_not_override_a_newer_answer() {
    let (mut tracker, start) = tracker();
    let older = tracker.begin(start);
    let end = answered_after(&mut tracker, start + ms(500), ms(200));
    tracker.finish(older, Outcome::Unreachable, end + ms(5000));
    assert_eq!(tracker.health(end + ms(5000)).level, Level::Good);
}

#[test]
fn a_request_started_after_the_last_answer_still_counts_as_waiting() {
    let (mut tracker, start) = tracker();
    let end = answered_after(&mut tracker, start, ms(200));
    let _newer = tracker.begin(end);
    assert_eq!(tracker.waiting_since(), Some(end));
    assert_eq!(tracker.health(end + ms(4000)).level, Level::Bad);
}

#[test]
fn a_request_started_while_another_was_in_flight_still_waits() {
    let (mut tracker, start) = tracker();
    let first = tracker.begin(start);
    let _second = tracker.begin(start + ms(100));
    tracker.finish(first, Outcome::Answered, start + ms(300));
    assert_eq!(tracker.waiting_since(), Some(start + ms(100)));
    let health = tracker.health(start + ms(4100));
    assert_eq!(
        (health.level, health.cause),
        (Level::Bad, Some(Cause::Waiting))
    );
}

#[test]
fn a_request_started_at_the_same_moment_is_not_superseded() {
    let (mut tracker, start) = tracker();
    let first = tracker.begin(start);
    let _second = tracker.begin(start);
    tracker.finish(first, Outcome::Answered, start + ms(200));
    assert_eq!(tracker.waiting_since(), Some(start));
}

#[test]
fn a_failure_does_not_count_toward_answer_latency() {
    let (mut tracker, start) = tracker();
    let ticket = tracker.begin(start);
    tracker.finish(ticket, Outcome::Unreachable, start + ms(6000));
    let end = answered_after(&mut tracker, start + ms(6000), ms(150));
    assert_eq!(
        tracker.health(end),
        ConnectionHealth {
            level: Level::Good,
            cause: None,
            latency_ms: Some(150),
        }
    );
}

#[test]
fn progress_restarts_the_wait() {
    let (mut tracker, start) = tracker();
    let ticket = tracker.begin(start);
    tracker.progress(&ticket, start + ms(3000));
    assert_eq!(tracker.waiting_since(), Some(start + ms(3000)));
    assert_eq!(tracker.health(start + ms(4000)).level, Level::Unknown);
    assert_eq!(
        tracker.next_change(start + ms(3000)),
        Some(start + ms(4500))
    );
    let stalled = tracker.health(start + ms(7000));
    assert_eq!(
        (stalled.level, stalled.cause),
        (Level::Bad, Some(Cause::Waiting))
    );
}

#[test]
fn concurrent_requests_answer_with_the_newest_start() {
    let (mut tracker, start) = tracker();
    let first = tracker.begin(start);
    let second = tracker.begin(start + ms(100));
    tracker.finish(second, Outcome::Answered, start + ms(2100));
    let health = tracker.health(start + ms(4500));
    assert_eq!(
        (health.level, health.cause),
        (Level::Slow, Some(Cause::SlowAnswers))
    );
    tracker.finish(first, Outcome::Answered, start + ms(9000));
    assert_eq!(tracker.health(start + ms(9000)).latency_ms, Some(2000));
}

#[test]
fn next_change_names_every_threshold_still_ahead() {
    let (mut tracker, start) = tracker();
    let end = answered_after(&mut tracker, start, ms(100));
    let _ticket = tracker.begin(end);
    assert_eq!(tracker.next_change(end), Some(end + THRESHOLDS.slow));
    assert_eq!(
        tracker.next_change(end + THRESHOLDS.slow),
        Some(end + THRESHOLDS.bad)
    );
    assert_eq!(
        tracker.next_change(end + THRESHOLDS.bad),
        Some(end + THRESHOLDS.memory)
    );
    assert_eq!(tracker.next_change(end + THRESHOLDS.memory), None);
}

#[test]
fn each_next_change_is_where_health_moves() {
    let (mut tracker, start) = tracker();
    let _ticket = tracker.begin(start);
    let mut now = start;
    let mut seen = vec![tracker.health(now).level];
    while let Some(moment) = tracker.next_change(now) {
        assert_ne!(tracker.health(moment - ms(1)), tracker.health(moment));
        now = moment;
        seen.push(tracker.health(now).level);
    }
    assert_eq!(seen, [Level::Unknown, Level::Slow, Level::Bad]);
}
