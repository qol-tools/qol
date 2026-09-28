use super::*;
use crate::session::{SessionGeneration, SessionNonce};

fn peer(value: u8) -> PeerId {
    PeerId::from_spki_digest([value; 32])
}

#[tokio::test(start_paused = true)]
async fn hints_are_bounded_trust_filtered_and_do_not_reset_backoff() {
    let id = peer(1);
    let trusted = BTreeSet::from([id]);
    let mut routes = Routes::default();
    let endpoints: Vec<_> = (1..=32)
        .map(|port| SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, port))
        .collect();
    routes.resolved("unknown".into(), peer(2), endpoints.clone(), &trusted, true);
    assert!(routes.peers.is_empty());
    routes.resolved("one".into(), id, endpoints.clone(), &trusted, true);
    assert_eq!(routes.peers[&id].hints.len(), 8);
    routes.peers.get_mut(&id).unwrap().failed();
    let deadline = routes.peers[&id].next;
    for _ in 0..50 {
        routes.resolved("one".into(), id, endpoints.clone(), &trusted, true);
    }
    assert_eq!(routes.peers[&id].next, deadline);
    routes.peers.get_mut(&id).unwrap().dialing = true;
    assert_eq!(routes.peers[&id].endpoint(), None);
    for attempt in 0..20 {
        routes.peers.get_mut(&id).unwrap().failed();
        assert!(
            routes.peers[&id].next <= Instant::now() + Duration::from_secs(30),
            "{attempt}"
        );
    }
    routes.removed("one");
    assert_eq!(routes.peers[&id].endpoint(), None);
    routes.retain(&BTreeSet::new());
    assert!(routes.peers.is_empty());
}

#[tokio::test]
async fn source_replacement_and_removal_preserve_other_sources() {
    let trusted = BTreeSet::from([peer(1), peer(2)]);
    let mut routes = Routes::default();
    let endpoint = SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 1234);
    routes.resolved("a".into(), peer(1), vec![endpoint], &trusted, true);
    routes.resolved("b".into(), peer(1), vec![endpoint], &trusted, true);
    routes.resolved("a".into(), peer(2), vec![endpoint], &trusted, true);
    routes.removed("a");
    assert_eq!(routes.peers[&peer(1)].endpoint(), Some(endpoint));
    assert_eq!(routes.peers[&peer(2)].endpoint(), None);
    routes.clear_hints();
    assert!(routes
        .peers
        .values()
        .all(|route| route.endpoint().is_none()));
}

#[test]
fn a_device_seen_before_it_was_linked_is_dialed_once_linked() {
    let mut routes = Routes::default();
    let endpoint = SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 1234);
    routes.resolved("a".into(), peer(1), vec![endpoint], &BTreeSet::new(), true);
    assert!(routes.peers.is_empty());
    routes.retain(&BTreeSet::from([peer(1)]));
    assert_eq!(routes.peers[&peer(1)].endpoint(), Some(endpoint));
    routes.removed("a");
    routes.retain(&BTreeSet::from([peer(1)]));
    assert_eq!(routes.peers[&peer(1)].endpoint(), None);
}

#[test]
fn endpoints_reject_unroutable_destinations_and_gate_loopback() {
    for (ip, port, loopback, accepted) in [
        ([0, 0, 0, 0], 80, false, false),
        ([224, 0, 0, 1], 80, false, false),
        ([255, 255, 255, 255], 80, false, false),
        ([192, 0, 2, 1], 0, false, false),
        ([127, 0, 0, 1], 80, false, false),
        ([127, 0, 0, 1], 80, true, true),
        ([192, 0, 2, 1], 80, false, true),
    ] {
        let address = SocketAddrV4::new(ip.into(), port);
        assert_eq!(
            valid(address, loopback),
            accepted,
            "{address} loopback={loopback}"
        );
    }
}

#[test]
fn both_ends_rank_the_same_initiator_and_nonce_pair() {
    let a = peer(1);
    let b = peer(2);
    for first in 0..8 {
        for second in 0..8 {
            let generation = SessionGeneration {
                local: SessionNonce::from_random([first; 16]),
                remote: SessionNonce::from_random([second; 16]),
            };
            let left = AuthenticatedSession {
                local_peer: a,
                remote_peer: b,
                generation,
            };
            let right = AuthenticatedSession {
                local_peer: b,
                remote_peer: a,
                generation: SessionGeneration {
                    local: generation.remote,
                    remote: generation.local,
                },
            };
            assert_eq!(rank(left, true), rank(right, false), "{first}/{second}");
            assert!(rank(left, true) < rank(left, false), "{first}/{second}");
        }
    }
}

#[tokio::test]
async fn hints_never_allocate_more_than_the_authority_peer_limit() {
    let trusted: BTreeSet<_> = (0..=256u16)
        .map(|index| {
            let mut bytes = [0; 32];
            bytes[..2].copy_from_slice(&index.to_be_bytes());
            PeerId::from_spki_digest(bytes)
        })
        .collect();
    let mut routes = Routes::default();
    for peer in &trusted {
        routes.resolved(
            peer.to_string(),
            *peer,
            vec![SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 1234)],
            &trusted,
            true,
        );
    }
    assert_eq!(routes.peers.len(), MAX_PEERS);
    assert!(routes
        .peers
        .values()
        .all(|route| route.hints.len() <= MAX_HINTS));
}
