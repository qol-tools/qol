use super::*;
use crate::enrollment::EnrollmentRejection;
use crate::service::enrollment::EnrollmentError;

#[tokio::test(start_paused = true)]
async fn expired_reservations_do_not_block_another_live_invitation() {
    for same_peer in [false, true] {
        let inviter = session("inviter");
        let joiner = session("joiner");
        let other = session("other");
        let first = inviter.create_invitation(vec![]).unwrap();
        let transaction = TransactionId::from_random([0; 16]);
        let first_request = request(&first, transaction, AuthorityLifetime::Session);
        inviter
            .reserve_enrollment(&joiner.local_pin().unwrap(), &first_request)
            .unwrap();
        let first_key = inviter.pending_enrollments().unwrap()[0].key;
        tokio::time::advance(Duration::from_secs(60)).await;
        let second = inviter.create_invitation(vec![]).unwrap();
        let (pin, transaction) = if same_peer {
            (
                joiner.local_pin().unwrap(),
                TransactionId::from_random([1; 16]),
            )
        } else {
            (other.local_pin().unwrap(), transaction)
        };
        let second_request = request(&second, transaction, AuthorityLifetime::Session);
        tokio::time::advance(Duration::from_secs(59)).await;
        assert_eq!(
            inviter.reserve_enrollment(&pin, &second_request),
            Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict)),
            "same_peer={same_peer}"
        );
        assert_eq!(revision(&inviter), StoreRevision::INITIAL);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(
            inviter.approve_enrollment(revision(&inviter), first_key),
            Err(EnrollmentError::Rejected(EnrollmentRejection::Expired))
        );
        inviter.reserve_enrollment(&pin, &second_request).unwrap();
        let pending = inviter.pending_enrollments().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].key.invitation, second.id());
        assert_eq!(pending[0].key.transaction, transaction);
        let receipt = inviter
            .approve_enrollment(revision(&inviter), pending[0].key)
            .unwrap();
        assert_eq!(receipt.invitation, second.id());
        assert_eq!(receipt.joiner, pin.peer_id());
        assert!(inviter.is_trusted(&pin));
        assert_eq!(revision(&inviter).value(), 1);
        let projection = inviter.projection().unwrap();
        assert_eq!(projection.peers.len(), 1);
        assert!(projection.peers[0].grants.is_empty());
        assert_eq!(inviter.inner.lock().unwrap().state.receipts.len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn expired_inbound_reservation_does_not_block_preparing_an_outbound_join() {
    let local = session("local");
    let remote = session("remote");
    let inbound = local.create_invitation(vec![]).unwrap();
    let request = request(
        &inbound,
        TransactionId::from_random([0; 16]),
        AuthorityLifetime::Session,
    );
    local
        .reserve_enrollment(&remote.local_pin().unwrap(), &request)
        .unwrap();
    let key = local.pending_enrollments().unwrap()[0].key;
    tokio::time::advance(Duration::from_secs(60)).await;
    let outbound = remote.create_invitation(vec![]).unwrap();
    tokio::time::advance(Duration::from_secs(59)).await;
    assert_eq!(
        local.prepare_join(revision(&local), &outbound),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict))
    );
    assert!(local.outbound_enrollments().unwrap().is_empty());
    assert_eq!(revision(&local), StoreRevision::INITIAL);
    tokio::time::advance(Duration::from_secs(1)).await;
    let transaction = local.prepare_join(revision(&local), &outbound).unwrap();
    let pending = local.outbound_enrollments().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].key.invitation, outbound.id());
    assert_eq!(pending[0].key.transaction, transaction);
    assert!(matches!(
        pending[0].state,
        OutboundEnrollmentState::Pending {}
    ));
    assert_eq!(
        local.approve_enrollment(revision(&local), key),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Expired))
    );
    assert!(local.projection().unwrap().peers.is_empty());
    assert!(!local.is_trusted(&remote.local_pin().unwrap()));
    assert_eq!(revision(&local).value(), 1);
}
