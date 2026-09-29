use super::*;
use crate::enrollment::EnrollmentRejection;
use crate::service::enrollment::EnrollmentError;
use crate::AuthorityError;

fn remote_receipt(
    inviter: &PeerAuthority,
    joiner: &PeerAuthority,
    invitation: &Invitation,
    transaction: TransactionId,
) -> EnrollmentReceipt {
    inviter
        .reserve_enrollment(
            &joiner.local_pin().unwrap(),
            &request(
                invitation,
                transaction,
                joiner.projection().unwrap().lifetime,
            ),
        )
        .unwrap();
    let key = inviter.pending_enrollments().unwrap()[0].key;
    inviter.approve_enrollment(revision(inviter), key).unwrap()
}

#[test]
fn lifecycle_transitions_check_revision_even_for_idempotent_requests() {
    let inviter = session("inviter");
    let joiner = session("joiner");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    for abandon in [false, true, true, false, false] {
        let before = revision(&joiner);
        let old = joiner.outbound_enrollments().unwrap()[0].clone();
        let stale = if abandon {
            joiner.abandon_join(StoreRevision::INITIAL, transaction)
        } else {
            joiner.resume_join(StoreRevision::INITIAL, transaction)
        };
        assert_eq!(
            stale,
            Err(EnrollmentError::Authority(AuthorityError::StaleRevision {
                expected: StoreRevision::INITIAL,
                current: before,
            })),
            "abandon={abandon}, state={:?}",
            old.state
        );
        let desired = if abandon {
            OutboundEnrollmentState::Abandoned {}
        } else {
            OutboundEnrollmentState::Pending {}
        };
        let result = if abandon {
            joiner.abandon_join(before, transaction)
        } else {
            joiner.resume_join(before, transaction)
        }
        .unwrap();
        assert_eq!(
            result.value(),
            before.value() + u64::from(old.state != desired)
        );
        let record = &joiner.outbound_enrollments().unwrap()[0];
        assert_eq!(record.key, old.key);
        assert_eq!(record.state, desired);
    }
    let receipt = remote_receipt(&inviter, &joiner, &invitation, transaction);
    joiner
        .commit_join(invitation.inviter_pin(), &receipt)
        .unwrap();
    let before = revision(&joiner);
    for result in [
        joiner.abandon_join(before, transaction),
        joiner.resume_join(before, transaction),
    ] {
        assert_eq!(
            result,
            Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict))
        );
    }
    assert_eq!(revision(&joiner), before);
    assert_eq!(
        joiner.outbound_enrollments().unwrap()[0].state,
        OutboundEnrollmentState::Committed { receipt }
    );
    let unknown = TransactionId::from_random([7; 16]);
    for result in [
        joiner.abandon_join(before, unknown),
        joiner.resume_join(before, unknown),
    ] {
        assert_eq!(
            result,
            Err(EnrollmentError::Rejected(
                EnrollmentRejection::UnknownTransaction
            ))
        );
    }
}

#[test]
fn abandon_and_commit_race_has_one_serialized_winner() {
    for _ in 0..8 {
        let inviter = session("inviter");
        let joiner = session("joiner");
        let invitation = inviter.create_invitation(vec![]).unwrap();
        let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
        let receipt = remote_receipt(&inviter, &joiner, &invitation, transaction);
        let before = revision(&joiner);
        let barrier = std::sync::Barrier::new(2);
        let (abandon, commit) = std::thread::scope(|scope| {
            let abandon = scope.spawn(|| {
                barrier.wait();
                joiner.abandon_join(before, transaction)
            });
            let commit = scope.spawn(|| {
                barrier.wait();
                joiner.commit_join(invitation.inviter_pin(), &receipt)
            });
            (abandon.join().unwrap(), commit.join().unwrap())
        });
        let after = revision(&joiner);
        assert_eq!(after.value(), before.value() + 1);
        match &joiner.outbound_enrollments().unwrap()[0].state {
            OutboundEnrollmentState::Abandoned {} => {
                assert_eq!(abandon, Ok(after));
                assert_eq!(commit, Err(EnrollmentError::Abandoned));
                assert!(!joiner.is_trusted(invitation.inviter_pin()));
            }
            OutboundEnrollmentState::Committed { receipt: stored } => {
                assert_eq!(stored, &receipt);
                assert_eq!(commit, Ok(()));
                assert_eq!(
                    abandon,
                    Err(EnrollmentError::Authority(AuthorityError::StaleRevision {
                        expected: before,
                        current: after
                    }))
                );
                assert!(joiner.is_trusted(invitation.inviter_pin()));
            }
            OutboundEnrollmentState::Pending {} => panic!("neither transition published"),
        }
    }
}

#[tokio::test]
async fn late_tls_receipt_cannot_resurrect_abandoned_join_but_explicit_resume_recovers() {
    let inviter = session("inviter");
    let joiner = session("joiner");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    let (client, mut server) = connections(&inviter, &joiner).await;
    let late = async {
        let request: Request = read_json(&mut server, FrameLimit::Enrollment)
            .await
            .unwrap();
        inviter
            .reserve_enrollment(server.remote_identity().pin(), &request)
            .unwrap();
        let receipt = approve(&inviter).await;
        joiner.abandon_join(revision(&joiner), transaction).unwrap();
        write_json(
            &mut server,
            &Response {
                version: EnrollmentVersion::V1,
                invitation: invitation.id(),
                transaction,
                outcome: ResponseOutcome::Committed { receipt },
            },
            FrameLimit::Enrollment,
        )
        .await
        .unwrap();
    };
    let (joined, ()) = tokio::join!(
        joiner.redeem_enrollment(client, &invitation, transaction),
        late
    );
    assert_eq!(
        joined.unwrap(),
        EnrollmentOutcome::Unknown {
            transaction,
            reason: EnrollmentError::Abandoned,
        }
    );
    assert!(joiner.projection().unwrap().peers.is_empty());
    assert!(inviter.is_trusted(&joiner.local_pin().unwrap()));
    for recover in [false, true] {
        let (client, server) = connections(&inviter, &joiner).await;
        let result = if recover {
            joiner.recover_enrollment(client, transaction).await
        } else {
            joiner
                .redeem_enrollment(client, &invitation, transaction)
                .await
        };
        assert_eq!(result, Err(EnrollmentError::Abandoned), "recover={recover}");
        drop(server);
    }
    joiner.resume_join(revision(&joiner), transaction).unwrap();
    drop(invitation);
    let (client, server) = connections(&inviter, &joiner).await;
    let (joined, accepted) = tokio::join!(
        joiner.recover_enrollment(client, transaction),
        inviter.serve_enrollment(server)
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
    assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
    assert_eq!(
        joiner.outbound_enrollments().unwrap()[0].key.transaction,
        transaction
    );
}

#[tokio::test(start_paused = true)]
async fn resume_refuses_live_incoming_and_competing_outgoing_but_accepts_expired_reservation() {
    let local = session("joiner");
    let remote = session("remote");
    let invitation = remote.create_invitation(vec![]).unwrap();
    let original = local.prepare_join(revision(&local), &invitation).unwrap();
    let inbound = local.create_invitation(vec![]).unwrap();
    let incoming = request(
        &inbound,
        TransactionId::from_random([1; 16]),
        AuthorityLifetime::Session,
    );
    let pin = remote.local_pin().unwrap();
    assert_eq!(
        local.reserve_enrollment(&pin, &incoming),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict))
    );
    local.abandon_join(revision(&local), original).unwrap();
    let reused = request(&inbound, original, AuthorityLifetime::Session);
    assert_eq!(
        local.reserve_enrollment(&pin, &reused),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict))
    );
    local.reserve_enrollment(&pin, &incoming).unwrap();
    let before = revision(&local);
    assert_eq!(
        local.resume_join(before, original),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict))
    );
    assert_eq!(revision(&local), before);
    tokio::time::advance(Duration::from_secs(120)).await;
    local.resume_join(before, original).unwrap();
    local.abandon_join(revision(&local), original).unwrap();
    let next = remote.create_invitation(vec![]).unwrap();
    let competing = local.prepare_join(revision(&local), &next).unwrap();
    assert_eq!(
        local.resume_join(revision(&local), original),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict))
    );
    local.abandon_join(revision(&local), competing).unwrap();
    local.resume_join(revision(&local), original).unwrap();
    assert_eq!(local.outbound_enrollments().unwrap().len(), 2);
}

#[test]
fn inbound_commit_can_coexist_with_abandoned_history_and_revocation_cleans_both() {
    let local = session("joiner");
    let remote = session("joiner");
    let invitation = remote.create_invitation(vec![]).unwrap();
    let original = local.prepare_join(revision(&local), &invitation).unwrap();
    local.abandon_join(revision(&local), original).unwrap();
    let inbound = local.create_invitation(vec![]).unwrap();
    let receipt = remote_receipt(
        &local,
        &remote,
        &inbound,
        TransactionId::from_random([1; 16]),
    );
    assert_eq!(
        local.resume_join(revision(&local), original),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict))
    );
    local.revoke(revision(&local), receipt.joiner).unwrap();
    assert!(local.outbound_enrollments().unwrap().is_empty());
    assert!(local.inner.lock().unwrap().state.receipts.is_empty());
    assert!(local.projection().unwrap().peers.is_empty());
    assert_eq!(
        local.resume_join(revision(&local), original),
        Err(EnrollmentError::Rejected(
            EnrollmentRejection::UnknownTransaction
        ))
    );
}

#[test]
fn abandoned_history_retains_unique_ids_and_counts_toward_retained_capacity() {
    let inviter = session("inviter");
    let joiner = session("joiner");
    let mut first = None;
    for index in 0..256 {
        let invitation = inviter.create_invitation(vec![]).unwrap();
        let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
        joiner.abandon_join(revision(&joiner), transaction).unwrap();
        assert_eq!(
            joiner.prepare_join(revision(&joiner), &invitation),
            Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict)),
            "{index}"
        );
        inviter.cancel_invitation(invitation.id()).unwrap();
        first.get_or_insert(transaction);
    }
    let before = revision(&joiner);
    let invitation = inviter.create_invitation(vec![]).unwrap();
    assert_eq!(
        joiner.prepare_join(before, &invitation),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Capacity))
    );
    assert_eq!(revision(&joiner), before);
    assert_eq!(joiner.outbound_enrollments().unwrap().len(), 256);
    joiner.resume_join(before, first.unwrap()).unwrap();
    assert_eq!(joiner.outbound_enrollments().unwrap().len(), 256);
}

#[test]
fn resume_pending_capacity_is_independent_of_retained_abandoned_records() {
    let joiner = session("joiner");
    let inviter = session("inviter");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let original = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    joiner.abandon_join(revision(&joiner), original).unwrap();
    let mut pending = Vec::new();
    for _ in 0..32 {
        let remote = session("remote");
        pending.push(
            joiner
                .prepare_join(
                    revision(&joiner),
                    &remote.create_invitation(vec![]).unwrap(),
                )
                .unwrap(),
        );
    }
    let before = revision(&joiner);
    assert_eq!(
        joiner.resume_join(before, original),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Capacity))
    );
    assert_eq!(revision(&joiner), before);
    joiner.abandon_join(before, pending[0]).unwrap();
    joiner.resume_join(revision(&joiner), original).unwrap();
    let records = joiner.outbound_enrollments().unwrap();
    assert_eq!(records.len(), 33);
    assert_eq!(
        records
            .iter()
            .filter(|entry| matches!(entry.state, OutboundEnrollmentState::Pending {}))
            .count(),
        32
    );
}
