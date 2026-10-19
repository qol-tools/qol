use std::time::{Duration, Instant};

use qol_connection_health::Level;

use super::service::next_probe;
use super::THRESHOLDS;

#[test]
fn a_first_look_probes_at_once() {
    let now = Instant::now();
    assert_eq!(next_probe(now, Level::Unknown, None, None), Some(now));
}

#[test]
fn no_probe_while_a_request_is_still_measuring() {
    let now = Instant::now();
    let just_started = Some(now);
    assert_eq!(next_probe(now, Level::Unknown, None, just_started), None);
    let almost_stalled = Some(now - THRESHOLDS.bad + Duration::from_millis(1));
    assert_eq!(
        next_probe(now, Level::Slow, Some(now), almost_stalled),
        None
    );
}

#[test]
fn a_stalled_request_lets_a_probe_check_the_connection() {
    let now = Instant::now() + THRESHOLDS.bad;
    let stalled = Some(now - THRESHOLDS.bad);
    assert_eq!(next_probe(now, Level::Bad, None, stalled), Some(now));
    let earlier = now - Duration::from_secs(30);
    let probe_at = next_probe(now, Level::Bad, Some(earlier), stalled).unwrap();
    assert!(probe_at <= now);
}

#[test]
fn a_probe_already_running_waits_for_its_turn() {
    let now = Instant::now() + THRESHOLDS.bad;
    let stalled = Some(now - THRESHOLDS.bad);
    let probe_at = next_probe(now, Level::Bad, Some(now), stalled).unwrap();
    assert!(probe_at - now > super::probe::TIMEOUT);
}

#[test]
fn a_good_connection_is_probed_rarely_and_a_doubtful_one_often() {
    let now = Instant::now();
    let good = next_probe(now, Level::Good, Some(now), None).unwrap();
    for level in [Level::Unknown, Level::Slow, Level::Bad] {
        let doubtful = next_probe(now, level, Some(now), None).unwrap();
        assert!(doubtful > now, "{level:?}");
        assert!(doubtful < good, "{level:?}");
    }
}

#[test]
fn probes_keep_a_watched_connection_remembered() {
    let now = Instant::now();
    let good = next_probe(now, Level::Good, Some(now), None).unwrap();
    assert!(good - now < THRESHOLDS.memory);
}

#[test]
fn thresholds_rise_from_slow_to_bad() {
    assert!(THRESHOLDS.slow < THRESHOLDS.bad);
    assert!(THRESHOLDS.bad < THRESHOLDS.memory);
    assert!(THRESHOLDS.bad < super::probe::TIMEOUT);
}
