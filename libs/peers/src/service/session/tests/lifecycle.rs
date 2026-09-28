use std::{future::pending, sync::atomic::Ordering, time::Duration};

use tokio::{
    io::AsyncWriteExt,
    time::{sleep, Instant},
};

use crate::service::framing::{read_idle_json, write_json, FrameLimit};

use super::{heartbeat, rename, revoke, Fixture, Message, SessionError, SessionOutcome};

#[tokio::test(start_paused = true)]
async fn valid_heartbeats_keep_both_sessions_alive_and_cancellation_drops_both_transports() {
    let fixture = Fixture::new();
    let (left, right) = fixture.sessions(64).await;
    let start = Instant::now();
    let (left, right) = tokio::join!(
        left.run(sleep(Duration::from_secs(121))),
        right.run(pending())
    );
    assert_eq!(left, SessionOutcome::Cancelled);
    assert_eq!(right, SessionOutcome::Closed(SessionError::Transport));
    assert_eq!(start.elapsed(), Duration::from_secs(121));
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn idle_peer_receives_only_periodic_heartbeats_and_expires_at_45_seconds() {
    let fixture = Fixture::new();
    let (session, mut remote, generation) = fixture.one_session().await;
    let start = Instant::now();
    let (outcome, beats) = tokio::join!(session.run(pending()), async {
        let mut beats = Vec::new();
        while let Ok(message) = read_idle_json::<_, Message>(&mut remote, FrameLimit::Normal).await
        {
            let expected = Message::Heartbeat {
                version: super::Version,
                sender_nonce: generation.local,
                recipient_nonce: generation.remote,
            };
            assert_eq!(message, expected);
            beats.push(start.elapsed());
        }
        beats
    });
    assert_eq!(outcome, SessionOutcome::Closed(SessionError::IdleExpired));
    assert_eq!(
        beats,
        vec![Duration::from_secs(15), Duration::from_secs(30)]
    );
    assert_eq!(start.elapsed(), Duration::from_secs(45));
    drop(remote);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn revoke_and_fault_interrupt_idle_sessions_and_release_both_ends() {
    for fault in [false, true] {
        let fixture = Fixture::new();
        let (left, right) = fixture.sessions(4096).await;
        let ((left, right), ()) = tokio::join!(
            async { tokio::join!(left.run(pending()), right.run(pending())) },
            async {
                sleep(Duration::from_secs(2)).await;
                if fault {
                    fixture.left.fault_for_session_test();
                    return;
                }
                revoke(&fixture.left, &fixture.right);
            }
        );
        let expected = if fault {
            SessionError::AuthorityUnavailable
        } else {
            SessionError::Untrusted
        };
        assert_eq!(left, SessionOutcome::Closed(expected), "fault {fault}");
        assert_eq!(
            right,
            SessionOutcome::Closed(SessionError::Transport),
            "fault {fault}"
        );
        assert_eq!(fixture.drops.load(Ordering::SeqCst), 2, "fault {fault}");
    }
}

#[tokio::test(start_paused = true)]
async fn dropping_run_future_cancels_all_work_and_releases_owned_transport() {
    let fixture = Fixture::new();
    let (left, right) = fixture.sessions(4096).await;
    let mut running = Box::pin(left.run(pending()));
    tokio::select! {
        result = &mut running => panic!("unexpected {result:?}"),
        () = sleep(Duration::from_secs(1)) => {},
    }
    drop(running);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        right.run(pending()).await,
        SessionOutcome::Closed(SessionError::Transport)
    );
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn writer_failure_ends_reader_before_idle_expiry() {
    let fixture = Fixture::new();
    let (session, remote, _) = fixture.one_session().await;
    fixture.fail_left_write.store(true, Ordering::SeqCst);
    let start = Instant::now();
    assert_eq!(
        session.run(pending()).await,
        SessionOutcome::Closed(SessionError::Transport)
    );
    assert_eq!(start.elapsed(), Duration::from_secs(15));
    drop(remote);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn blocked_writer_deadline_ends_reader_and_releases_transport() {
    let fixture = Fixture::new();
    let (session, remote, _) = fixture.one_session().await;
    fixture.stall_left_write.store(true, Ordering::SeqCst);
    let start = Instant::now();
    assert_eq!(
        session.run(pending()).await,
        SessionOutcome::Closed(SessionError::FrameTimeout)
    );
    assert_eq!(start.elapsed(), Duration::from_secs(20));
    drop(remote);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn missed_heartbeat_ticks_do_not_burst_when_run_starts_late() {
    let fixture = Fixture::new();
    let (session, mut remote, _) = fixture.one_session().await;
    sleep(Duration::from_secs(40)).await;
    let start = Instant::now();
    let (outcome, received) = tokio::join!(session.run(sleep(Duration::from_secs(4))), async {
        let mut received = Vec::new();
        while read_idle_json::<_, Message>(&mut remote, FrameLimit::Normal)
            .await
            .is_ok()
        {
            received.push(start.elapsed());
        }
        received
    });
    assert_eq!(outcome, SessionOutcome::Cancelled);
    assert_eq!(received, vec![Duration::ZERO]);
}

#[tokio::test(start_paused = true)]
async fn inbound_control_rate_window_recovers_while_heartbeats_preserve_liveness() {
    let fixture = Fixture::new();
    let (session, mut remote, generation) = fixture.one_session().await;
    let start = Instant::now();
    let (outcome, ()) = tokio::join!(session.run(sleep(Duration::from_secs(61))), async {
        for (second, count) in [(0, 63), (30, 1), (60, 63)] {
            tokio::time::sleep_until(start + Duration::from_secs(second)).await;
            for _ in 0..count {
                write_json(&mut remote, &heartbeat(generation), FrameLimit::Normal)
                    .await
                    .unwrap();
            }
        }
    });
    assert_eq!(outcome, SessionOutcome::Cancelled);
}

#[tokio::test(start_paused = true)]
async fn inbound_wrong_direction_old_generation_and_repeated_hello_close_session() {
    for mode in ["direction", "old", "hello"] {
        let fixture = Fixture::new();
        let (old, old_remote, old_generation) = fixture.one_session().await;
        drop((old, old_remote));
        let (session, mut remote, generation) = fixture.one_session().await;
        let message = match mode {
            "direction" => Message::Heartbeat {
                version: super::Version,
                sender_nonce: generation.local,
                recipient_nonce: generation.remote,
            },
            "old" => heartbeat(old_generation),
            "hello" => super::wire::fixtures().0,
            _ => unreachable!(),
        };
        let (outcome, ()) = tokio::join!(session.run(pending()), async {
            write_json(&mut remote, &message, FrameLimit::Normal)
                .await
                .unwrap();
        });
        let expected = if mode == "hello" {
            SessionError::Protocol
        } else {
            SessionError::Generation
        };
        assert_eq!(outcome, SessionOutcome::Closed(expected), "{mode}");
        drop(remote);
        assert_eq!(fixture.drops.load(Ordering::SeqCst), 4, "{mode}");
    }
}

#[tokio::test(start_paused = true)]
async fn control_rate_accepts_64_and_rejects_65th_without_idle_reset_by_mutations() {
    for count in [64, 65] {
        let fixture = Fixture::new();
        let (session, mut remote, generation) = fixture.one_session().await;
        let (outcome, ()) = tokio::join!(session.run(sleep(Duration::from_secs(1))), async {
            for _ in 0..count {
                write_json(&mut remote, &heartbeat(generation), FrameLimit::Normal)
                    .await
                    .unwrap();
            }
            rename(&fixture.left);
        });
        let expected = if count == 64 {
            SessionOutcome::Cancelled
        } else {
            SessionOutcome::Closed(SessionError::RateExceeded)
        };
        assert_eq!(outcome, expected, "{count}");
    }
}

#[tokio::test(start_paused = true)]
async fn malformed_and_oversized_tls_control_frames_close_immediately() {
    for payload in [
        Vec::new(),
        b"{}".to_vec(),
        b"{\"kind\":\"heartbeat\",\"kind\":\"heartbeat\"}".to_vec(),
        vec![0xff],
        vec![b' '; 65537],
    ] {
        let fixture = Fixture::new();
        let (session, mut remote, _) = fixture.one_session().await;
        let (outcome, ()) = tokio::join!(session.run(pending()), async {
            remote
                .write_all(&(payload.len() as u32).to_be_bytes())
                .await
                .unwrap();
            if payload.len() <= 65536 {
                remote.write_all(&payload).await.unwrap();
            }
            remote.flush().await.unwrap();
        });
        assert_eq!(
            outcome,
            SessionOutcome::Closed(SessionError::Protocol),
            "length {}",
            payload.len()
        );
    }
}
