use super::*;
use crate::network::DiscoveryStatus;

#[tokio::test]
async fn simultaneous_real_tls_dials_converge_and_discovery_removal_does_not_disconnect() {
    let (left, right) = linked();
    let left_id = left.local_pin().unwrap().peer_id();
    let right_id = right.local_pin().unwrap().peer_id();
    let mut a = Fixture::start(left.clone());
    let mut b = Fixture::start(right);
    let a_port = a.port().await;
    let b_port = b.port().await;
    a.events.send(hint(right_id, b_port)).await.unwrap();
    b.events.send(hint(left_id, a_port)).await.unwrap();
    a.wait(|view| view.sessions.len() == 1).await;
    b.wait(|view| view.sessions.len() == 1).await;
    let mut a_view = a.control.subscribe();
    let mut b_view = b.control.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let a_session = a_view.borrow().sessions.first().copied();
            let b_session = b_view.borrow().sessions.first().copied();
            if let (Some(a_session), Some(b_session)) = (a_session, b_session) {
                if a_session.generation.local == b_session.generation.remote
                    && a_session.generation.remote == b_session.generation.local
                {
                    break;
                }
            }
            tokio::select! {
                result = a_view.changed() => result.unwrap(),
                result = b_view.changed() => result.unwrap(),
            }
        }
    })
    .await
    .unwrap();
    let a_session = a.control.snapshot().sessions[0];
    let b_session = b.control.snapshot().sessions[0];
    assert_eq!(a_session.generation.local, b_session.generation.remote);
    assert_eq!(a_session.generation.remote, b_session.generation.local);
    for _ in 0..20 {
        a.events.send(hint(right_id, b_port)).await.unwrap();
    }
    a.events
        .send(DiscoveryEvent::Removed {
            source: right_id.to_string(),
        })
        .await
        .unwrap();
    a.events.send(DiscoveryEvent::Ready).await.unwrap();
    a.wait(|view| view.status.discovery == DiscoveryStatus::Ready {})
        .await;
    assert_eq!(a.control.snapshot().sessions.len(), 1);
    let revision = a.control.snapshot().revision;
    left.revoke(left.projection().unwrap().revision, right_id)
        .unwrap();
    a.wait(|view| view.sessions.is_empty() && view.revision != revision)
        .await;
    b.wait(|view| view.sessions.is_empty()).await;
    a.stop().await;
    b.stop().await;
}

#[tokio::test]
async fn authority_fault_closes_real_tls_sessions_and_preserves_the_authority() {
    let (left, right) = linked();
    let right_id = right.local_pin().unwrap().peer_id();
    let mut a = Fixture::start(left.clone());
    let mut b = Fixture::start(right);
    a.port().await;
    let port = b.port().await;
    a.events.send(hint(right_id, port)).await.unwrap();
    a.wait(|view| view.sessions.len() == 1).await;
    left.fault_for_session_test();
    assert_eq!(a.task.await.unwrap(), Err(NetworkFailure::Authority));
    assert_eq!(left.projection().unwrap().peers.len(), 1);
    assert!(a.control.snapshot().sessions.is_empty());
    b.stop().await;
}

#[tokio::test]
async fn revoke_during_tcp_handshake_reaps_the_attempt_without_authenticating() {
    let (left, right) = linked();
    let peer = right.local_pin().unwrap().peer_id();
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let mut owner = Fixture::start(left.clone());
    owner.port().await;
    owner
        .events
        .send(hint(peer, listener.local_addr().unwrap().port()))
        .await
        .unwrap();
    let (mut socket, _) = listener.accept().await.unwrap();
    left.revoke(left.projection().unwrap().revision, peer)
        .unwrap();
    use tokio::io::AsyncReadExt;
    let mut buffer = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut buffer))
        .await
        .unwrap();
    assert!(owner.control.snapshot().sessions.is_empty());
    owner.stop().await;
}

#[tokio::test]
async fn repeated_hints_and_renames_cannot_replace_a_single_authenticated_session() {
    let (left, right) = linked();
    let right_id = right.local_pin().unwrap().peer_id();
    let mut a = Fixture::start(left.clone());
    let mut b = Fixture::start(right);
    a.port().await;
    let port = b.port().await;
    a.events.send(hint(right_id, port)).await.unwrap();
    a.wait(|view| view.sessions.len() == 1).await;
    b.wait(|view| view.sessions.len() == 1).await;
    let initial = a.control.snapshot();
    for _ in 0..20 {
        a.events.send(hint(right_id, port)).await.unwrap();
    }
    left.rename(left.projection().unwrap().revision, "renamed".into())
        .unwrap();
    a.events.send(DiscoveryEvent::Ready).await.unwrap();
    a.wait(|view| view.status.discovery == DiscoveryStatus::Ready {})
        .await;
    assert_eq!(a.control.snapshot().revision, initial.revision);
    assert_eq!(a.control.snapshot().sessions, initial.sessions);
    a.stop().await;
    b.stop().await;
}
