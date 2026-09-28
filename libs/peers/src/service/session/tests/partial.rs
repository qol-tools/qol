use std::{future::pending, time::Duration};

use tokio::{
    io::AsyncWriteExt,
    time::{sleep, sleep_until, Instant},
};

use crate::service::framing::{read_idle_json, write_json, FrameLimit};

use super::{
    frame, heartbeat, rename, Fixture, Message, NormalSession, SessionError, SessionOutcome,
};

#[tokio::test(start_paused = true)]
async fn partial_prefix_and_payload_survive_authority_notifications_and_outbound_heartbeat() {
    for split_at in [1, 3, 4, 9] {
        let fixture = Fixture::new();
        let (session, mut remote, generation) = fixture.one_session().await;
        let bytes = frame(&heartbeat(generation));
        let start = Instant::now();
        let (outcome, ()) = tokio::join!(session.run(sleep(Duration::from_secs(18))), async {
            sleep_until(start + Duration::from_secs(14)).await;
            remote.write_all(&bytes[..split_at]).await.unwrap();
            remote.flush().await.unwrap();
            sleep_until(start + Duration::from_secs(15)).await;
            rename(&fixture.left);
            let beat: Message = read_idle_json(&mut remote, FrameLimit::Normal)
                .await
                .unwrap();
            assert!(matches!(beat, Message::Heartbeat { .. }));
            sleep_until(start + Duration::from_secs(16)).await;
            remote.write_all(&bytes[split_at..]).await.unwrap();
            remote.flush().await.unwrap();
            write_json(&mut remote, &heartbeat(generation), FrameLimit::Normal)
                .await
                .unwrap();
        });
        assert_eq!(outcome, SessionOutcome::Cancelled, "split {split_at}");
    }
}

#[tokio::test(start_paused = true)]
async fn authority_changes_and_trickle_do_not_restart_five_second_partial_frame_deadline() {
    for split_at in [1, 3, 4, 9] {
        let fixture = Fixture::new();
        let (session, mut remote, generation) = fixture.one_session().await;
        let bytes = frame(&heartbeat(generation));
        let start = Instant::now();
        let (outcome, ()) = tokio::join!(session.run(pending()), async {
            sleep_until(start + Duration::from_secs(2)).await;
            remote.write_all(&bytes[..split_at]).await.unwrap();
            remote.flush().await.unwrap();
            for second in 3..=6 {
                sleep_until(start + Duration::from_secs(second)).await;
                rename(&fixture.left);
            }
            remote
                .write_all(&bytes[split_at..split_at + 1])
                .await
                .unwrap();
            remote.flush().await.unwrap();
        });
        assert_eq!(
            outcome,
            SessionOutcome::Closed(SessionError::FrameTimeout),
            "split {split_at}"
        );
        assert_eq!(start.elapsed(), Duration::from_secs(7), "split {split_at}");
    }
}

#[tokio::test(start_paused = true)]
async fn quiet_periods_over_five_seconds_between_inbound_frames_are_valid() {
    let fixture = Fixture::new();
    let (session, mut remote, generation) = fixture.one_session().await;
    let start = Instant::now();
    let (outcome, ()) = tokio::join!(session.run(sleep(Duration::from_secs(44))), async {
        for second in [10, 25, 40] {
            sleep_until(start + Duration::from_secs(second)).await;
            write_json(&mut remote, &heartbeat(generation), FrameLimit::Normal)
                .await
                .unwrap();
        }
    });
    assert_eq!(outcome, SessionOutcome::Cancelled);
}

#[tokio::test(start_paused = true)]
async fn hello_deadline_includes_quiet_time_and_partial_prefix() {
    for first_byte in [None, Some(4)] {
        let fixture = Fixture::new();
        let (left, mut right) = fixture.connections(4096).await;
        let start = Instant::now();
        let (result, ()) = tokio::join!(NormalSession::open(left, fixture.left.clone()), async {
            if let Some(second) = first_byte {
                sleep(Duration::from_secs(second)).await;
                right.write_all(&[0]).await.unwrap();
                right.flush().await.unwrap();
            }
        });
        assert_eq!(
            result.err(),
            Some(SessionError::HelloTimeout),
            "{first_byte:?}"
        );
        assert_eq!(start.elapsed(), Duration::from_secs(5), "{first_byte:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn cancellation_during_partial_read_drops_transport_without_restarting_reader() {
    let fixture = Fixture::new();
    let (session, mut remote, _) = fixture.one_session().await;
    let (outcome, ()) = tokio::join!(session.run(sleep(Duration::from_secs(2))), async {
        remote.write_all(&[0, 0]).await.unwrap();
        remote.flush().await.unwrap();
        sleep(Duration::from_secs(1)).await;
        rename(&fixture.left);
    });
    assert_eq!(outcome, SessionOutcome::Cancelled);
}
