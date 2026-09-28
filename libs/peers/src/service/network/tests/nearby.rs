use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
};

use super::*;

fn peer(byte: u8) -> PeerId {
    PeerId::from_spki_digest([byte; 32])
}

fn claim(name: &str) -> Option<NearbyAdvertisement> {
    Some(NearbyAdvertisement {
        name: name.into(),
        link: 7000,
    })
}

fn lan(last: u8) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, last), 5000)
}

#[test]
fn a_claim_lists_the_computer_on_its_link_port() {
    let mut nearby = Nearby::default();
    assert!(nearby.resolved(
        "desk".into(),
        peer(1),
        &[lan(2)],
        claim("Desk"),
        &BTreeSet::new(),
        false
    ));
    assert_eq!(
        nearby.claims(),
        vec![NearbyClaim {
            peer_id: peer(1),
            name: "Desk".into(),
            endpoints: vec![SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::new(192, 168, 1, 2),
                7000
            ))],
        }]
    );
}

#[test]
fn linked_revoked_local_and_unclaimed_computers_are_not_listed() {
    let mut nearby = Nearby::default();
    let ignored = BTreeSet::from([peer(1)]);
    assert!(!nearby.resolved(
        "a".into(),
        peer(1),
        &[lan(2)],
        claim("Linked"),
        &ignored,
        false
    ));
    assert!(!nearby.resolved("b".into(), peer(2), &[lan(3)], None, &ignored, false));
    assert!(!nearby.resolved(
        "c".into(),
        peer(3),
        &[SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5000)],
        claim("Loopback"),
        &ignored,
        false
    ));
    assert!(nearby.claims().is_empty());
    assert!(nearby.resolved(
        "d".into(),
        peer(4),
        &[lan(4)],
        claim("Desk"),
        &ignored,
        false
    ));
    assert!(nearby.retain(&BTreeSet::from([peer(4)])));
    assert!(nearby.claims().is_empty());
}

#[test]
fn removal_and_failure_drop_claims_and_the_list_is_bounded() {
    let mut nearby = Nearby::default();
    for index in 0..=MAX_NEARBY as u8 {
        nearby.resolved(
            format!("source-{index}"),
            peer(index + 1),
            &[lan(index + 1)],
            claim("Desk"),
            &BTreeSet::new(),
            false,
        );
    }
    assert_eq!(nearby.claims().len(), MAX_NEARBY);
    assert!(nearby.removed("source-0"));
    assert_eq!(nearby.claims().len(), MAX_NEARBY - 1);
    assert!(nearby.clear());
    assert!(nearby.claims().is_empty());
}

#[test]
fn two_sources_for_one_computer_merge_their_endpoints() {
    let mut nearby = Nearby::default();
    nearby.resolved(
        "wifi".into(),
        peer(1),
        &[lan(2)],
        claim("Desk"),
        &BTreeSet::new(),
        false,
    );
    nearby.resolved(
        "wired".into(),
        peer(1),
        &[lan(3)],
        claim("Desk"),
        &BTreeSet::new(),
        false,
    );
    let claims = nearby.claims();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].endpoints.len(), 2);
}
