use super::{frame, heartbeat, rename, Fixture, Message, SessionError, SessionOutcome, Version};
use crate::{
    operations::OperationEpoch,
    service::{
        framing::{read_idle_json, write_json, FrameLimit},
        network::operations::Hub,
    },
};
use std::{future::pending, time::Duration};
use tokio::{
    io::AsyncWriteExt,
    time::{sleep, sleep_until, Instant},
};

fn epoch(fixture: &Fixture, generation: crate::session::SessionGeneration) -> Message {
    Message::OperationEpoch {
        version: Version,
        sender_nonce: generation.remote,
        recipient_nonce: generation.local,
        epoch: fixture
            .right
            .operation_epoch(fixture.left.local_pin().unwrap().peer_id())
            .unwrap(),
    }
}

#[tokio::test(start_paused = true)]
async fn operation_epoch_fragment_survives_heartbeat_and_authority_notifications() {
    for split_at in [1, 3, 4, 17] {
        let fixture = Fixture::new();
        let (session, mut remote, generation) = fixture.one_session().await;
        let authenticated = session.authenticated().unwrap();
        let (hub, _workers) = Hub::new(fixture.left.clone(), None);
        let io = hub.elect(authenticated).unwrap();
        let bytes = frame(&epoch(&fixture, generation));
        let start = Instant::now();
        let (outcome, ()) = tokio::join!(
            session.run_operations(sleep(Duration::from_secs(18)), hub.clone(), io),
            async {
                let advertised: Message = read_idle_json(&mut remote, FrameLimit::Normal)
                    .await
                    .unwrap();
                assert!(matches!(advertised, Message::OperationEpoch { .. }));
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
            }
        );
        assert_eq!(outcome, SessionOutcome::Cancelled);
        assert!(hub.ready(authenticated.remote_peer));
    }
}

#[tokio::test]
async fn operation_epoch_requires_elected_nonces_and_single_exchange() {
    for stale in [false, true] {
        let fixture = Fixture::new();
        let (session, mut remote, generation) = fixture.one_session().await;
        let (hub, _workers) = Hub::new(fixture.left.clone(), None);
        let io = hub.elect(session.authenticated().unwrap()).unwrap();
        let mut message = epoch(&fixture, generation);
        if stale {
            let Message::OperationEpoch {
                recipient_nonce, ..
            } = &mut message
            else {
                unreachable!()
            };
            *recipient_nonce = crate::session::SessionNonce::from_random([99; 16]);
        }
        let (outcome, ()) = tokio::join!(session.run_operations(pending(), hub, io), async {
            let _: Message = read_idle_json(&mut remote, FrameLimit::Normal)
                .await
                .unwrap();
            write_json(&mut remote, &message, FrameLimit::Normal)
                .await
                .unwrap();
            if !stale {
                write_json(&mut remote, &message, FrameLimit::Normal)
                    .await
                    .unwrap();
            }
        });
        assert_eq!(
            outcome,
            SessionOutcome::Closed(if stale {
                SessionError::Generation
            } else {
                SessionError::Protocol
            })
        );
    }
}

#[test]
fn operation_epoch_wire_keeps_nonce_encoding() {
    let epoch = OperationEpoch(crate::session::SessionNonce::from_random([0; 16]));
    assert_eq!(
        serde_json::to_string(&epoch).unwrap(),
        "\"AAAAAAAAAAAAAAAAAAAAAA\""
    );
}
