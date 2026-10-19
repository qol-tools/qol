use std::time::Instant;

use qol_connection_health::Level;

use super::service::next_probe;
use super::THRESHOLDS;

#[test]
fn a_first_look_probes_at_once() {
    let now = Instant::now();
    assert_eq!(next_probe(now, Level::Unknown, None, false), Some(now));
}

#[test]
fn no_probe_while_a_request_is_already_measuring() {
    let now = Instant::now();
    assert_eq!(next_probe(now, Level::Unknown, None, true), None);
    assert_eq!(next_probe(now, Level::Bad, Some(now), true), None);
}

#[test]
fn a_good_connection_is_probed_rarely_and_a_doubtful_one_often() {
    let now = Instant::now();
    let good = next_probe(now, Level::Good, Some(now), false).unwrap();
    for level in [Level::Unknown, Level::Slow, Level::Bad] {
        let doubtful = next_probe(now, level, Some(now), false).unwrap();
        assert!(doubtful > now, "{level:?}");
        assert!(doubtful < good, "{level:?}");
    }
}

#[test]
fn probes_keep_a_watched_connection_remembered() {
    let now = Instant::now();
    let good = next_probe(now, Level::Good, Some(now), false).unwrap();
    assert!(good - now < THRESHOLDS.memory);
}

#[test]
fn thresholds_rise_from_slow_to_bad() {
    assert!(THRESHOLDS.slow < THRESHOLDS.bad);
    assert!(THRESHOLDS.bad < THRESHOLDS.memory);
    assert!(THRESHOLDS.bad < super::probe::TIMEOUT);
}
