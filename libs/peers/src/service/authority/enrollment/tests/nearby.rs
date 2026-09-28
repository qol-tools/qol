use std::net::SocketAddr;

use qol_conventions::{
    operations::{OperationKey, OperationKind},
    plugin_id::PluginUid,
};

use super::*;
use crate::enrollment::EnrollmentRejection;
use crate::service::enrollment::{EnrollmentError, NearbyOffer};

fn endpoint() -> SocketAddr {
    "192.168.1.10:7000".parse().unwrap()
}

fn grant(name: &str) -> OperationKey {
    OperationKey::new(
        PluginUid::new("bluetooth-fixture-uid"),
        OperationKind::Action,
        name,
    )
}

async fn nearby(
    inviter: &PeerAuthority,
    joiner: &PeerAuthority,
) -> (
    Result<NearbyOffer, EnrollmentError>,
    Result<(), EnrollmentError>,
) {
    let client = joiner
        .nearby_client_config(inviter.local_pin().unwrap().peer_id())
        .unwrap();
    let server = inviter.enrollment_server_config().unwrap();
    let (client_io, server_io) = tokio::io::duplex(16 * 1024);
    let (client, server) = tokio::join!(client.connect(client_io), server.accept(server_io));
    tokio::join!(
        joiner.request_nearby(client.unwrap()),
        inviter.serve_nearby(server.unwrap(), endpoint())
    )
}

async fn redeem(
    inviter: &PeerAuthority,
    joiner: &PeerAuthority,
    offer: &NearbyOffer,
    transaction: TransactionId,
) -> EnrollmentOutcome {
    let (client, server) = connections(inviter, joiner).await;
    let (joined, _) = tokio::join!(
        joiner.redeem_enrollment(client, &offer.invitation, transaction),
        inviter.serve_enrollment(server)
    );
    joined.unwrap()
}

#[tokio::test]
async fn both_computers_show_one_code_and_an_early_confirmation_links_with_its_grants() {
    let inviter = session("Laptop");
    let joiner = session("Desk");
    let (offer, served) = nearby(&inviter, &joiner).await;
    served.unwrap();
    let offer = offer.unwrap();
    let requests = inviter.inbound_nearby().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].code, offer.code);
    assert_eq!(requests[0].name, "Desk");
    assert_eq!(offer.invitation.endpoints(), [endpoint()]);
    assert!(inviter.pending_enrollments().unwrap().is_empty());

    let peer = joiner.local_pin().unwrap().peer_id();
    inviter
        .confirm_nearby(revision(&inviter), peer, vec![grant("release")])
        .unwrap();
    assert!(inviter.inbound_nearby().unwrap()[0].confirmed);

    let transaction = joiner
        .prepare_nearby_join(revision(&joiner), &offer.invitation)
        .unwrap();
    let EnrollmentOutcome::Completed(receipt) =
        redeem(&inviter, &joiner, &offer, transaction).await
    else {
        panic!("nearby link did not complete");
    };
    joiner
        .grant_linked(receipt.inviter, vec![grant("resume")])
        .unwrap();
    let laptop = inviter.projection().unwrap();
    assert_eq!(laptop.peers[0].peer_id, peer);
    assert_eq!(laptop.peers[0].grants, vec![grant("release")]);
    let desk = joiner.projection().unwrap();
    assert_eq!(desk.peers[0].name, "Laptop");
    assert_eq!(desk.peers[0].grants, vec![grant("resume")]);
    assert!(inviter.inbound_nearby().unwrap().is_empty());
}

#[tokio::test]
async fn a_late_confirmation_approves_the_waiting_redemption() {
    let inviter = session("Laptop");
    let joiner = session("Desk");
    let offer = nearby(&inviter, &joiner).await.0.unwrap();
    let transaction = joiner
        .prepare_nearby_join(revision(&joiner), &offer.invitation)
        .unwrap();
    let peer = joiner.local_pin().unwrap().peer_id();
    let confirming = async {
        let mut events = inviter.watch_enrollment();
        while !inviter.inbound_nearby().unwrap()[0].redeemed {
            events.changed().await.unwrap();
        }
        inviter
            .confirm_nearby(revision(&inviter), peer, Vec::new())
            .unwrap();
    };
    let (outcome, ()) = tokio::join!(redeem(&inviter, &joiner, &offer, transaction), confirming);
    assert!(matches!(outcome, EnrollmentOutcome::Completed(_)));
    assert_eq!(joiner.projection().unwrap().peers.len(), 1);
    assert_eq!(inviter.projection().unwrap().peers[0].grants, Vec::new());
}

#[tokio::test]
async fn another_key_cannot_redeem_a_nearby_invitation() {
    let inviter = session("Laptop");
    let joiner = session("Desk");
    let stranger = session("Desk");
    let offer = nearby(&inviter, &joiner).await.0.unwrap();
    let transaction = stranger
        .prepare_join(revision(&stranger), &offer.invitation)
        .unwrap();
    let outcome = redeem(&inviter, &stranger, &offer, transaction).await;
    assert_eq!(
        outcome,
        EnrollmentOutcome::Rejected(EnrollmentRejection::Conflict)
    );
    assert!(inviter.projection().unwrap().peers.is_empty());
}

#[tokio::test]
async fn declining_cancels_the_request_before_the_joiner_confirms() {
    let inviter = session("Laptop");
    let joiner = session("Desk");
    let offer = nearby(&inviter, &joiner).await.0.unwrap();
    let peer = joiner.local_pin().unwrap().peer_id();
    inviter.decline_nearby(revision(&inviter), peer).unwrap();
    assert!(inviter.inbound_nearby().unwrap().is_empty());
    let transaction = joiner
        .prepare_nearby_join(revision(&joiner), &offer.invitation)
        .unwrap();
    let outcome = redeem(&inviter, &joiner, &offer, transaction).await;
    assert_eq!(
        outcome,
        EnrollmentOutcome::Rejected(EnrollmentRejection::InvalidInvitation)
    );
}

#[tokio::test]
async fn a_linked_computer_is_refused_and_a_new_nearby_join_replaces_a_stale_one() {
    let inviter = session("Laptop");
    let joiner = session("Desk");
    let stale = nearby(&inviter, &joiner).await.0.unwrap();
    let old = joiner
        .prepare_nearby_join(revision(&joiner), &stale.invitation)
        .unwrap();
    let offer = nearby(&inviter, &joiner).await.0.unwrap();
    assert_eq!(inviter.inbound_nearby().unwrap().len(), 1);
    let transaction = joiner
        .prepare_nearby_join(revision(&joiner), &offer.invitation)
        .unwrap();
    let states: Vec<_> = joiner
        .outbound_enrollments()
        .unwrap()
        .into_iter()
        .map(|join| (join.key.transaction, join.state))
        .collect();
    assert_eq!(
        states,
        vec![
            (old, OutboundEnrollmentState::Abandoned {}),
            (transaction, OutboundEnrollmentState::Pending {}),
        ]
    );
    let peer = joiner.local_pin().unwrap().peer_id();
    inviter
        .confirm_nearby(revision(&inviter), peer, Vec::new())
        .unwrap();
    redeem(&inviter, &joiner, &offer, transaction).await;
    let (again, served) = nearby(&inviter, &joiner).await;
    assert_eq!(
        served,
        Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict))
    );
    assert!(again.is_err());
}

#[tokio::test]
async fn a_stale_join_toward_the_joiner_does_not_block_linking_nearby() {
    let inviter = session("Laptop");
    let joiner = session("Desk");
    let earlier = nearby(&joiner, &inviter).await.0.unwrap();
    let stale = inviter
        .prepare_nearby_join(revision(&inviter), &earlier.invitation)
        .unwrap();
    let offer = nearby(&inviter, &joiner).await.0.unwrap();
    let peer = joiner.local_pin().unwrap().peer_id();
    inviter
        .confirm_nearby(revision(&inviter), peer, Vec::new())
        .unwrap();
    let transaction = joiner
        .prepare_nearby_join(revision(&joiner), &offer.invitation)
        .unwrap();
    let outcome = redeem(&inviter, &joiner, &offer, transaction).await;
    assert!(matches!(outcome, EnrollmentOutcome::Completed(_)));
    assert_eq!(inviter.projection().unwrap().peers[0].peer_id, peer);
    let states: Vec<_> = inviter
        .outbound_enrollments()
        .unwrap()
        .into_iter()
        .map(|join| (join.key.transaction, join.state))
        .collect();
    assert_eq!(states, vec![(stale, OutboundEnrollmentState::Abandoned {})]);
}
