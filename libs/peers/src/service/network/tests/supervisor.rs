use super::*;
use crate::{
    network::NetworkStatus,
    session::{SessionGeneration, SessionNonce},
};

fn owner() -> Supervisor {
    let authority = PeerAuthority::session("local".into(), std::time::SystemTime::now()).unwrap();
    let view = watch::channel(NetworkSnapshot {
        revision: NetworkRevision::default(),
        status: NetworkStatus::default(),
        sessions: Vec::new(),
        nearby: Vec::new(),
    })
    .0;
    let (operations, workers) = super::super::operations::Hub::new(authority.clone(), None);
    Supervisor {
        authority,
        operations,
        workers,
        dispatches: JoinSet::new(),
        trusted: BTreeSet::new(),
        ignored: BTreeSet::new(),
        routes: Routes::default(),
        nearby: super::super::nearby::Nearby::default(),
        live: BTreeMap::new(),
        handshakes: JoinSet::new(),
        sessions: JoinSet::new(),
        view,
        loopback: true,
    }
}

#[tokio::test]
async fn stale_session_exit_compares_both_nonces_and_keeps_the_replacement() {
    let mut owner = owner();
    let remote = PeerId::from_spki_digest([1; 32]);
    let current = AuthenticatedSession {
        local_peer: owner.authority.local_pin().unwrap().peer_id(),
        remote_peer: remote,
        generation: SessionGeneration {
            local: SessionNonce::from_random([2; 16]),
            remote: SessionNonce::from_random([3; 16]),
        },
    };
    owner.live.insert(
        remote,
        Live {
            authenticated: current,
            outgoing: true,
            stop: watch::channel(false).0,
        },
    );
    owner.publish_sessions().unwrap();
    let revision = owner.view.borrow().revision;
    for generation in [
        SessionGeneration {
            local: SessionNonce::from_random([4; 16]),
            ..current.generation
        },
        SessionGeneration {
            remote: SessionNonce::from_random([4; 16]),
            ..current.generation
        },
    ] {
        owner
            .exited(AuthenticatedSession {
                generation,
                ..current
            })
            .unwrap();
        assert_eq!(owner.view.borrow().revision, revision, "{generation:?}");
        assert_eq!(owner.live[&remote].authenticated, current, "{generation:?}");
    }
    owner.exited(current).unwrap();
    assert!(owner.live.is_empty());
    assert_ne!(owner.view.borrow().revision, revision);
}

#[tokio::test]
async fn concurrent_dials_are_capped_and_repeated_scheduling_cannot_duplicate_them() {
    let mut owner = owner();
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let trusted: BTreeSet<_> = (1..=32)
        .map(|index| PeerId::from_spki_digest([index; 32]))
        .collect();
    for peer in &trusted {
        owner.routes.resolved(
            peer.to_string(),
            *peer,
            vec![SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, port)],
            &trusted,
            true,
        );
    }
    owner.dial_ready();
    assert_eq!(owner.handshakes.len(), MAX_HANDSHAKES);
    assert_eq!(
        owner
            .routes
            .peers
            .values()
            .filter(|route| route.dialing)
            .count(),
        MAX_HANDSHAKES
    );
    owner.dial_ready();
    assert_eq!(owner.handshakes.len(), MAX_HANDSHAKES);
    assert_eq!(owner.next_retry(), None);
    owner.handshakes.abort_all();
    while owner.handshakes.join_next().await.is_some() {}
}

#[tokio::test]
async fn accept_budget_is_checked_before_the_next_handshake_can_be_spawned() {
    let mut owner = owner();
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let mut sockets = Vec::new();
    for index in 0..MAX_HANDSHAKES {
        let client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (server, _) = listener.accept().await.unwrap();
        assert!(owner.can_accept(), "{index}");
        owner.accept(server);
        sockets.push(client);
    }
    assert!(!owner.can_accept());
    owner.handshakes.abort_all();
    while owner.handshakes.join_next().await.is_some() {}
    assert!(owner.can_accept());
    drop(sockets);
}

#[tokio::test]
async fn discovery_failure_invalidates_routing_without_clearing_authenticated_sessions() {
    let mut owner = owner();
    let peer = PeerId::from_spki_digest([1; 32]);
    owner.trusted.insert(peer);
    owner.discovery_event(DiscoveryEvent::Resolved {
        source: "peer".into(),
        peer,
        endpoints: vec![SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 4242)],
        claim: None,
    });
    assert!(owner.routes.peers[&peer].endpoint().is_some());
    let authenticated = AuthenticatedSession {
        local_peer: owner.authority.local_pin().unwrap().peer_id(),
        remote_peer: peer,
        generation: SessionGeneration {
            local: SessionNonce::from_random([2; 16]),
            remote: SessionNonce::from_random([3; 16]),
        },
    };
    owner.live.insert(
        peer,
        Live {
            authenticated,
            outgoing: false,
            stop: watch::channel(false).0,
        },
    );
    owner.publish_sessions().unwrap();
    let revision = owner.view.borrow().revision;
    owner.discovery_event(DiscoveryEvent::Failed);
    assert_eq!(owner.routes.peers[&peer].endpoint(), None);
    assert_eq!(owner.next_retry(), None);
    assert_eq!(owner.view.borrow().sessions, vec![authenticated]);
    assert_eq!(owner.view.borrow().revision, revision);
    assert_eq!(
        owner.view.borrow().status.discovery,
        DiscoveryStatus::Failed {
            error: NetworkFailure::Discovery
        }
    );
}
