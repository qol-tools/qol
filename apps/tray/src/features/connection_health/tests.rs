use std::time::{Duration, Instant};

use qol_connection_health::{Cause, ConnectionHealth, Level, Outcome, Tracker};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::body::ObservedBody;
use super::service::{next_probe, step, Step};
use super::{observe, Shared, INTEREST_WINDOW, THRESHOLDS};

fn isolated() -> &'static Shared {
    Box::leak(Box::new(Shared::new()))
}

async fn serve(reply: &'static [u8], then_stall: bool) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 1024];
        let _ = socket.read(&mut request).await;
        socket.write_all(reply).await.unwrap();
        if then_stall {
            std::future::pending::<()>().await;
        }
    });
    url
}

async fn closed_port() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    url
}

fn health(shared: &Shared) -> ConnectionHealth {
    shared.tracker().health(Instant::now())
}

#[test]
fn a_first_look_probes_at_once() {
    let now = Instant::now();
    assert_eq!(next_probe(now, Level::Unknown, None, None), Some(now));
}

#[test]
fn no_probe_while_a_request_is_still_measuring() {
    let now = Instant::now() + THRESHOLDS.bad;
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

#[test]
fn a_changed_signal_is_published_once() {
    let now = Instant::now();
    let mut tracker = Tracker::new(THRESHOLDS);
    let ticket = tracker.begin(now);
    tracker.finish(ticket, Outcome::Answered, now + Duration::from_millis(100));
    let later = now + Duration::from_millis(100);
    let first = step(&tracker, later, ConnectionHealth::default(), None, false);
    let health = first.publish.expect("good differs from unknown");
    assert_eq!(health.level, Level::Good);
    let again = step(&tracker, later, health, None, false);
    assert_eq!(again.publish, None);
}

#[test]
fn nothing_is_probed_unless_someone_is_interested() {
    let now = Instant::now();
    let tracker = Tracker::new(THRESHOLDS);
    let unwatched = step(&tracker, now, ConnectionHealth::default(), None, false);
    assert_eq!(
        unwatched,
        Step {
            publish: None,
            probe: false,
            deadline: None,
        }
    );
    let watched = step(&tracker, now, ConnectionHealth::default(), None, true);
    assert!(watched.probe);
    assert_eq!(watched.deadline, Some(now));
}

#[test]
fn the_loop_sleeps_until_the_next_change_or_probe() {
    let now = Instant::now();
    let mut tracker = Tracker::new(THRESHOLDS);
    let _waiting = tracker.begin(now);
    let measuring = step(&tracker, now, ConnectionHealth::default(), None, true);
    assert!(!measuring.probe);
    assert_eq!(measuring.deadline, Some(now + THRESHOLDS.slow));

    let mut tracker = Tracker::new(THRESHOLDS);
    let ticket = tracker.begin(now);
    tracker.finish(ticket, Outcome::Answered, now);
    let good = tracker.health(now);
    let settled = step(&tracker, now, good, None, true);
    assert!(!settled.probe);
    let probe_at = next_probe(now, Level::Good, Some(now), None).unwrap();
    assert!(probe_at < now + THRESHOLDS.memory);
    assert_eq!(settled.deadline, Some(probe_at));
}

#[test]
fn interest_lasts_one_window() {
    let shared = isolated();
    let now = Instant::now();
    assert!(!shared.watching(now));
    shared.note_interest(now);
    assert!(shared.watching(now + INTEREST_WINDOW - Duration::from_millis(1)));
    assert!(!shared.watching(now + INTEREST_WINDOW));
}

#[tokio::test]
async fn any_status_is_an_answer() {
    let shared = isolated();
    let url = serve(
        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
        false,
    )
    .await;
    let response = observe(shared, reqwest::Client::new().get(url)).await;
    assert_eq!(response.unwrap().status(), 503);
    assert_eq!(health(shared).level, Level::Good);
    assert_eq!(shared.tracker().waiting_since(), None);
}

#[tokio::test]
async fn a_closed_port_is_unreachable() {
    let shared = isolated();
    let url = closed_port().await;
    let error = observe(shared, reqwest::Client::new().get(url))
        .await
        .unwrap_err();
    assert!(super::is_unreachable(&error), "{error:?}");
    let health = health(shared);
    assert_eq!(
        (health.level, health.cause),
        (Level::Bad, Some(Cause::Unreachable))
    );
}

#[tokio::test]
async fn a_request_that_could_not_be_built_is_not_counted() {
    let shared = isolated();
    let error = observe(shared, reqwest::Client::new().get("not a url"))
        .await
        .unwrap_err();
    assert!(!super::is_unreachable(&error), "{error:?}");
    assert_eq!(shared.tracker().last_observed(), None);
    assert_eq!(shared.tracker().waiting_since(), None);
}

#[tokio::test]
async fn a_cancelled_request_leaves_nothing_waiting() {
    let shared = isolated();
    let url = serve(b"", true).await;
    let mut request = Box::pin(observe(shared, reqwest::Client::new().get(url)));
    tokio::select! {
        _ = &mut request => panic!("the server never answers"),
        _ = tokio::time::sleep(Duration::from_millis(100)) => {}
    }
    assert!(shared.tracker().waiting_since().is_some());
    drop(request);
    assert_eq!(shared.tracker().waiting_since(), None);
    assert_eq!(shared.tracker().last_observed(), None);
}

#[tokio::test]
async fn a_body_that_stalls_counts_as_waiting() {
    let shared = isolated();
    let url = serve(
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n0123456789",
        true,
    )
    .await;
    let response = observe(shared, reqwest::Client::new().get(url))
        .await
        .unwrap();
    assert_eq!(shared.tracker().waiting_since(), None);
    let mut body = ObservedBody::on(shared, response);
    let first = body.chunk().await.unwrap().expect("the first ten bytes");
    assert_eq!(first.as_ref(), b"0123456789");
    let progressed = shared
        .tracker()
        .waiting_since()
        .expect("the body is watched");
    let stalled = tokio::time::timeout(Duration::from_millis(100), body.chunk()).await;
    assert!(stalled.is_err());
    assert_eq!(shared.tracker().waiting_since(), Some(progressed));
    let waiting = shared.tracker().health(progressed + THRESHOLDS.bad);
    assert_eq!(
        (waiting.level, waiting.cause),
        (Level::Bad, Some(Cause::Waiting))
    );
    drop(body);
    assert_eq!(shared.tracker().waiting_since(), None);
}

#[tokio::test]
async fn a_body_cut_short_is_unreachable() {
    let shared = isolated();
    let url = serve(
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n0123456789",
        false,
    )
    .await;
    let response = observe(shared, reqwest::Client::new().get(url))
        .await
        .unwrap();
    let error = ObservedBody::on(shared, response)
        .bytes()
        .await
        .unwrap_err();
    assert!(super::is_unreachable(&error), "{error:?}");
    let health = health(shared);
    assert_eq!(
        (health.level, health.cause),
        (Level::Bad, Some(Cause::Unreachable))
    );
}

#[tokio::test]
async fn a_whole_body_leaves_nothing_waiting() {
    let shared = isolated();
    let url = serve(
        b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n0123456789",
        false,
    )
    .await;
    let response = observe(shared, reqwest::Client::new().get(url))
        .await
        .unwrap();
    let body = ObservedBody::on(shared, response).bytes().await.unwrap();
    assert_eq!(body, b"0123456789");
    assert_eq!(shared.tracker().waiting_since(), None);
    assert_eq!(health(shared).level, Level::Good);
}

#[tokio::test]
async fn a_body_that_times_out_is_unreachable() {
    let shared = isolated();
    let url = serve(
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n0123456789",
        true,
    )
    .await;
    let client = reqwest::Client::builder()
        .read_timeout(Duration::from_millis(100))
        .build()
        .unwrap();
    let response = observe(shared, client.get(url)).await.unwrap();
    let error = ObservedBody::on(shared, response)
        .bytes()
        .await
        .unwrap_err();
    assert!(super::is_unreachable(&error), "{error:?}");
    assert_eq!(health(shared).cause, Some(Cause::Unreachable));
}
