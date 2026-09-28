use std::fs;

use super::*;
use crate::enrollment::EnrollmentRejection;
use crate::service::authority::{storage::CommitFault, AuthorityError};
use crate::service::enrollment::EnrollmentError;
use crate::AuthorityStatus;

fn persistent(name: &str) -> (tempfile::TempDir, std::path::PathBuf, PeerAuthority) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("authority");
    let authority = PeerAuthority::create_persistent(&root, name.into(), now()).unwrap();
    (temporary, root, authority)
}

#[test]
fn abandon_and_resume_writer_faults_require_reopen_and_preserve_original_evidence() {
    for abandon in [true, false] {
        for (fault, replaced) in [
            (CommitFault::BeforeReplace, false),
            (CommitFault::AfterReplace, true),
        ] {
            let (_temporary, root, joiner) = persistent("joiner");
            let inviter = session("inviter");
            let invitation = inviter.create_invitation(vec![]).unwrap();
            let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
            if !abandon {
                joiner.abandon_join(revision(&joiner), transaction).unwrap();
            }
            let original = joiner.inner.lock().unwrap().state.outbound[0].clone();
            let before = revision(&joiner);
            joiner.inner.lock().unwrap().storage.fail_next(fault);
            let result = if abandon {
                joiner.abandon_join(before, transaction)
            } else {
                joiner.resume_join(before, transaction)
            };
            assert_eq!(
                result,
                Err(EnrollmentError::Authority(AuthorityError::Storage))
            );
            assert_eq!(
                joiner.projection().unwrap().status,
                AuthorityStatus::Faulted
            );
            assert_eq!(
                joiner.outbound_enrollments(),
                Err(EnrollmentError::Authority(AuthorityError::Faulted))
            );
            assert_eq!(
                joiner.resume_join(before, transaction),
                Err(EnrollmentError::Authority(AuthorityError::Faulted))
            );
            assert_eq!(
                joiner.abandon_join(before, transaction),
                Err(EnrollmentError::Authority(AuthorityError::Faulted))
            );
            assert_eq!(
                joiner.inner.lock().unwrap().state.outbound[0].state,
                original.state
            );
            drop(joiner);
            let reopened = PeerAuthority::open_persistent(&root, now()).unwrap();
            let record = reopened.inner.lock().unwrap().state.outbound[0].clone();
            assert_eq!(record.invitation, original.invitation);
            assert_eq!(record.transaction, original.transaction);
            assert_eq!(record.pin, original.pin);
            assert_eq!(record.name, original.name);
            assert_eq!(record.local_lifetime, original.local_lifetime);
            assert_eq!(record.remote_lifetime, original.remote_lifetime);
            let expected = if abandon == replaced {
                OutboundEnrollmentState::Abandoned {}
            } else {
                OutboundEnrollmentState::Pending {}
            };
            assert_eq!(
                record.state, expected,
                "abandon={abandon}, replaced={replaced}"
            );
            assert_eq!(
                revision(&reopened).value(),
                before.value() + u64::from(replaced)
            );
        }
    }
}

#[tokio::test]
async fn restart_preserves_abandoned_history_beside_new_join_and_committed_link() {
    let (_temporary, root, joiner) = persistent("joiner");
    let inviter = session("inviter");
    let original_invitation = inviter.create_invitation(vec![]).unwrap();
    let original = joiner
        .prepare_join(revision(&joiner), &original_invitation)
        .unwrap();
    joiner.abandon_join(revision(&joiner), original).unwrap();
    let evidence = joiner.outbound_enrollments().unwrap()[0].clone();
    drop(joiner);
    let joiner = PeerAuthority::open_persistent(&root, now()).unwrap();
    assert_eq!(
        joiner.outbound_enrollments().unwrap(),
        vec![evidence.clone()]
    );
    assert_eq!(
        joiner.prepare_join(revision(&joiner), &original_invitation),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict))
    );
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    drop(joiner);
    let joiner = PeerAuthority::open_persistent(&root, now()).unwrap();
    assert_eq!(
        joiner.resume_join(revision(&joiner), original),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict))
    );
    let (client, server) = connections(&inviter, &joiner).await;
    let (joined, accepted, _) = tokio::join!(
        joiner.redeem_enrollment(client, &invitation, transaction),
        inviter.serve_enrollment(server),
        approve(&inviter)
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
    assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
    drop(joiner);
    let joiner = PeerAuthority::open_persistent(&root, now()).unwrap();
    let records = joiner.outbound_enrollments().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0], evidence);
    assert!(matches!(
        records[1].state,
        OutboundEnrollmentState::Committed { .. }
    ));
    assert_eq!(
        joiner.resume_join(revision(&joiner), original),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict))
    );
    joiner
        .revoke(revision(&joiner), invitation.inviter_pin().peer_id())
        .unwrap();
    drop(joiner);
    let joiner = PeerAuthority::open_persistent(&root, now()).unwrap();
    assert!(joiner.outbound_enrollments().unwrap().is_empty());
    assert_eq!(
        joiner.projection().unwrap().tombstones,
        vec![invitation.inviter_pin().peer_id()]
    );
}

#[tokio::test]
async fn abandoned_unknown_remote_commit_survives_restart_and_recovers_only_after_resume() {
    let (_temporary, root, joiner) = persistent("joiner");
    let inviter = session("inviter");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    inviter
        .reserve_enrollment(
            &joiner.local_pin().unwrap(),
            &request(&invitation, transaction, AuthorityLifetime::Persistent),
        )
        .unwrap();
    let receipt = approve(&inviter).await;
    joiner.abandon_join(revision(&joiner), transaction).unwrap();
    assert_eq!(
        joiner.commit_join(invitation.inviter_pin(), &receipt),
        Err(EnrollmentError::Abandoned)
    );
    let snapshot: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("state.json")).unwrap()).unwrap();
    assert_eq!(snapshot["version"], 4);
    assert_eq!(
        snapshot["outbound"][0]["state"],
        serde_json::json!({"kind":"abandoned"})
    );
    assert!(snapshot["outbound"][0].get("receipt").is_none());
    drop((joiner, invitation));
    let joiner = PeerAuthority::open_persistent(&root, now()).unwrap();
    let (client, server) = connections(&inviter, &joiner).await;
    assert_eq!(
        joiner.recover_enrollment(client, transaction).await,
        Err(EnrollmentError::Abandoned)
    );
    drop(server);
    joiner.resume_join(revision(&joiner), transaction).unwrap();
    drop(joiner);
    let joiner = PeerAuthority::open_persistent(&root, now()).unwrap();
    let (client, server) = connections(&inviter, &joiner).await;
    let (joined, accepted) = tokio::join!(
        joiner.recover_enrollment(client, transaction),
        inviter.serve_enrollment(server)
    );
    assert_eq!(
        joined.unwrap(),
        EnrollmentOutcome::Completed(receipt.clone())
    );
    assert_eq!(accepted.unwrap(), EnrollmentOutcome::Completed(receipt));
}

#[test]
fn snapshot_requires_explicit_state_and_rejects_duplicate_ids_and_active_attempts() {
    let (_temporary, root, joiner) = persistent("joiner");
    let inviter = session("inviter");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let first = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    joiner.abandon_join(revision(&joiner), first).unwrap();
    let invitation = inviter.create_invitation(vec![]).unwrap();
    joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    drop(joiner);
    let fixture: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("state.json")).unwrap()).unwrap();
    for case in [
        "missing_state",
        "null_state",
        "unknown_state",
        "legacy_receipt",
        "old_version",
        "pending_extra",
        "abandoned_extra",
        "committed_missing_receipt",
        "duplicate_state",
        "duplicate_kind",
        "escaped_duplicate_kind",
        "duplicate_transaction",
        "duplicate_invitation",
        "two_active",
        "abandoned_with_receipt",
    ] {
        let mut value = fixture.clone();
        match case {
            "missing_state" => {
                value["outbound"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("state");
            }
            "null_state" => value["outbound"][0]["state"] = serde_json::Value::Null,
            "unknown_state" => {
                value["outbound"][0]["state"]["kind"] = serde_json::json!("cancelled")
            }
            "legacy_receipt" => {
                value["outbound"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("state");
                value["outbound"][0]["receipt"] = serde_json::Value::Null;
            }
            "old_version" => value["version"] = serde_json::json!(2),
            "pending_extra" => value["outbound"][1]["state"]["extra"] = serde_json::json!(true),
            "abandoned_extra" => value["outbound"][0]["state"]["extra"] = serde_json::json!(true),
            "committed_missing_receipt" => {
                value["outbound"][0]["state"]["kind"] = serde_json::json!("committed")
            }
            "duplicate_transaction" => {
                value["outbound"][1]["transaction"] = value["outbound"][0]["transaction"].clone()
            }
            "duplicate_invitation" => {
                value["outbound"][1]["invitation"] = value["outbound"][0]["invitation"].clone()
            }
            "two_active" => value["outbound"][0]["state"]["kind"] = serde_json::json!("pending"),
            "abandoned_with_receipt" => {
                value["outbound"][0]["state"]["receipt"] = serde_json::Value::Null
            }
            "duplicate_state" | "duplicate_kind" | "escaped_duplicate_kind" => {}
            other => panic!("unknown case {other}"),
        }
        let raw = serde_json::to_string(&value).unwrap();
        let raw = match case {
            "duplicate_state" => raw.replacen(
                r#""state":{"kind":"abandoned"}"#,
                r#""state":{"kind":"abandoned"},"state":{"kind":"pending"}"#,
                1,
            ),
            "duplicate_kind" => raw.replacen(
                r#""kind":"abandoned""#,
                r#""kind":"abandoned","kind":"pending""#,
                1,
            ),
            "escaped_duplicate_kind" => raw.replacen(
                r#""kind":"abandoned""#,
                r#""kind":"abandoned","\u006bind":"pending""#,
                1,
            ),
            _ => raw,
        };
        fs::write(root.join("state.json"), &raw).unwrap();
        assert!(
            PeerAuthority::open_persistent(&root, now()).is_err(),
            "{case}"
        );
        assert_eq!(
            fs::read_to_string(root.join("state.json")).unwrap(),
            raw,
            "{case}"
        );
    }
}

#[tokio::test]
async fn pending_outbound_write_faults_preserve_original_intent_only_if_replaced() {
    for (fault, replaced) in [
        (CommitFault::BeforeReplace, false),
        (CommitFault::AfterReplace, true),
    ] {
        let (_temporary, root, joiner) = persistent("joiner");
        let inviter = session("inviter");
        let invitation = inviter.create_invitation(vec![]).unwrap();
        joiner.inner.lock().unwrap().storage.fail_next(fault);
        assert_eq!(
            joiner.prepare_join(revision(&joiner), &invitation),
            Err(EnrollmentError::Authority(AuthorityError::Storage))
        );
        assert_eq!(
            joiner.projection().unwrap().status,
            AuthorityStatus::Faulted
        );
        assert!(inviter.pending_enrollments().unwrap().is_empty());
        drop(joiner);
        let reopened = PeerAuthority::open_persistent(&root, now()).unwrap();
        let outbound = reopened.outbound_enrollments().unwrap();
        assert_eq!(outbound.len(), usize::from(replaced));
        if replaced {
            assert_eq!(outbound[0].key.invitation, invitation.id());
            assert!(matches!(
                outbound[0].state,
                OutboundEnrollmentState::Pending {}
            ));
        }
    }
}

#[tokio::test]
async fn approval_write_faults_deny_live_recovery_and_reconcile_durable_receipt_after_reopen() {
    for (fault, replaced) in [
        (CommitFault::BeforeReplace, false),
        (CommitFault::AfterReplace, true),
    ] {
        let (_temporary, root, inviter) = persistent("inviter");
        let joiner = session("joiner");
        let invitation = inviter.create_invitation(vec![]).unwrap();
        let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
        let (client, server) = connections(&inviter, &joiner).await;
        let approval = async {
            let mut events = inviter.watch_enrollment();
            loop {
                if let Some(request) = inviter.pending_enrollments().unwrap().first() {
                    inviter.inner.lock().unwrap().storage.fail_next(fault);
                    assert_eq!(
                        inviter.approve_enrollment(revision(&inviter), request.key),
                        Err(EnrollmentError::Authority(AuthorityError::Storage))
                    );
                    assert_eq!(
                        inviter.projection().unwrap().status,
                        AuthorityStatus::Faulted
                    );
                    assert!(!inviter.is_trusted(&joiner.local_pin().unwrap()));
                    break;
                }
                events.changed().await.unwrap();
            }
        };
        let (joined, accepted, ()) = tokio::join!(
            joiner.redeem_enrollment(client, &invitation, transaction),
            inviter.serve_enrollment(server),
            approval
        );
        assert!(matches!(joined.unwrap(), EnrollmentOutcome::Unknown { .. }));
        assert!(matches!(
            accepted.unwrap(),
            EnrollmentOutcome::Unknown {
                reason: EnrollmentError::Authority(AuthorityError::Faulted),
                ..
            }
        ));
        drop(inviter);
        let inviter = PeerAuthority::open_persistent(&root, now()).unwrap();
        assert_eq!(inviter.is_trusted(&joiner.local_pin().unwrap()), replaced);
        let (client, server) = connections(&inviter, &joiner).await;
        let (joined, accepted) = tokio::join!(
            joiner.recover_enrollment(client, transaction),
            inviter.serve_enrollment(server)
        );
        if replaced {
            assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
            assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
        } else {
            assert!(matches!(
                joined.unwrap(),
                EnrollmentOutcome::Unknown {
                    reason: EnrollmentError::Rejected(
                        crate::enrollment::EnrollmentRejection::UnknownTransaction
                    ),
                    ..
                }
            ));
            assert!(matches!(
                accepted.unwrap(),
                EnrollmentOutcome::Unknown {
                    reason: EnrollmentError::Rejected(
                        crate::enrollment::EnrollmentRejection::UnknownTransaction
                    ),
                    ..
                }
            ));
            assert!(inviter.projection().unwrap().peers.is_empty());
        }
    }
}

#[tokio::test]
async fn lost_joiner_persistence_result_recovers_same_transaction_before_or_after_replace() {
    for (fault, replaced) in [
        (CommitFault::BeforeReplace, false),
        (CommitFault::AfterReplace, true),
    ] {
        let inviter = session("inviter");
        let (_temporary, root, joiner) = persistent("joiner");
        let invitation = inviter.create_invitation(vec![]).unwrap();
        let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
        let (client, server) = connections(&inviter, &joiner).await;
        joiner.inner.lock().unwrap().storage.fail_next(fault);
        let (joined, accepted, _) = tokio::join!(
            joiner.redeem_enrollment(client, &invitation, transaction),
            inviter.serve_enrollment(server),
            approve(&inviter)
        );
        assert!(matches!(
            joined.unwrap(),
            EnrollmentOutcome::Unknown {
                reason: EnrollmentError::Authority(AuthorityError::Storage),
                ..
            }
        ));
        assert!(matches!(
            accepted.unwrap(),
            EnrollmentOutcome::Unknown { .. }
        ));
        assert_eq!(
            joiner.projection().unwrap().status,
            AuthorityStatus::Faulted
        );
        assert!(!joiner.is_trusted(&inviter.local_pin().unwrap()));
        drop(joiner);
        let joiner = PeerAuthority::open_persistent(&root, now()).unwrap();
        assert_eq!(joiner.is_trusted(&inviter.local_pin().unwrap()), replaced);
        assert_eq!(
            joiner.outbound_enrollments().unwrap()[0].key.transaction,
            transaction
        );
        assert_eq!(
            matches!(
                joiner.outbound_enrollments().unwrap()[0].state,
                OutboundEnrollmentState::Committed { .. }
            ),
            replaced
        );
        let (client, server) = connections(&inviter, &joiner).await;
        let (joined, accepted) = tokio::join!(
            joiner.recover_enrollment(client, transaction),
            inviter.serve_enrollment(server)
        );
        assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
        assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
        assert_eq!(joiner.projection().unwrap().peers.len(), 1);
        assert_eq!(
            joiner.outbound_enrollments().unwrap()[0].key.transaction,
            transaction
        );
    }
}

#[tokio::test]
async fn faults_and_revocation_after_reservation_are_observed_before_success() {
    for revoke in [false, true] {
        let (_temporary, root, inviter) = persistent("inviter");
        let joiner = session("joiner");
        let invitation = inviter.create_invitation(vec![]).unwrap();
        let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
        let (mut client, server) = connections(&inviter, &joiner).await;
        write_json(
            &mut client,
            &request(&invitation, transaction, AuthorityLifetime::Session),
            FrameLimit::Enrollment,
        )
        .await
        .unwrap();
        let (accepted, receipt) = tokio::join!(
            inviter.serve_enrollment(server),
            approve_then_invalidate(&inviter, revoke)
        );
        let expected = if revoke {
            EnrollmentOutcome::Rejected(EnrollmentRejection::Revoked)
        } else {
            EnrollmentOutcome::Unknown {
                transaction,
                reason: EnrollmentError::Authority(AuthorityError::Faulted),
            }
        };
        assert_eq!(accepted.unwrap(), expected, "revoke={revoke}");
        loop {
            let response = read_json::<_, Response>(&mut client, FrameLimit::Enrollment).await;
            if let Ok(Response {
                outcome: ResponseOutcome::Pending {},
                ..
            }) = &response
            {
                continue;
            }
            if revoke {
                let response = response.unwrap();
                assert_eq!(response.invitation, invitation.id());
                assert_eq!(response.transaction, transaction);
                assert!(matches!(
                    response.outcome,
                    ResponseOutcome::Rejected {
                        reason: EnrollmentRejection::Revoked
                    }
                ));
                break;
            }
            assert!(response.is_err());
            break;
        }
        drop(client);
        assert!(!inviter.is_trusted(&joiner.local_pin().unwrap()));
        assert!(joiner.projection().unwrap().peers.is_empty());
        let outbound = joiner.outbound_enrollments().unwrap();
        assert_eq!(outbound.len(), 1);
        assert_eq!(outbound[0].key.transaction, transaction);
        assert!(matches!(
            outbound[0].state,
            OutboundEnrollmentState::Pending {}
        ));
        if revoke {
            assert!(inviter.projection().unwrap().peers.is_empty());
            drop(inviter);
            let reopened = PeerAuthority::open_persistent(&root, now()).unwrap();
            assert!(reopened.projection().unwrap().peers.is_empty());
            assert!(reopened.inner.lock().unwrap().state.receipts.is_empty());
            assert_eq!(
                reopened.projection().unwrap().tombstones,
                vec![joiner.local_pin().unwrap().peer_id()]
            );
            continue;
        }
        let projection = inviter.projection().unwrap();
        assert_eq!(projection.status, AuthorityStatus::Faulted);
        assert_eq!(projection.name, "inviter");
        assert_eq!(projection.revision.value(), 1);
        assert_eq!(projection.peers.len(), 1);
        assert!(projection.peers[0].grants.is_empty());
        assert_eq!(
            inviter.inner.lock().unwrap().state.receipts[0].receipt,
            receipt
        );
        drop(inviter);
        let reopened = PeerAuthority::open_persistent(&root, now()).unwrap();
        assert_eq!(reopened.projection().unwrap().name, "faulted");
        assert_eq!(reopened.projection().unwrap().revision.value(), 2);
        assert!(reopened.is_trusted(&joiner.local_pin().unwrap()));
        assert_eq!(
            reopened.inner.lock().unwrap().state.receipts[0].receipt,
            receipt
        );
    }
}

async fn approve_then_invalidate(inviter: &PeerAuthority, revoke: bool) -> EnrollmentReceipt {
    let receipt = approve(inviter).await;
    if revoke {
        inviter.revoke(revision(inviter), receipt.joiner).unwrap();
        return receipt;
    }
    inviter
        .inner
        .lock()
        .unwrap()
        .storage
        .fail_next(CommitFault::AfterReplace);
    assert_eq!(
        inviter.rename(revision(inviter), "faulted".into()),
        Err(AuthorityError::Storage)
    );
    receipt
}

#[tokio::test]
async fn disconnect_after_reservation_never_reports_completion() {
    for revoke in [false, true] {
        let (_temporary, _root, inviter) = persistent("inviter");
        let joiner = session("joiner");
        let invitation = inviter.create_invitation(vec![]).unwrap();
        let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
        let (mut client, server) = connections(&inviter, &joiner).await;
        write_json(
            &mut client,
            &request(&invitation, transaction, AuthorityLifetime::Session),
            FrameLimit::Enrollment,
        )
        .await
        .unwrap();
        let disconnect = async {
            approve_then_invalidate(&inviter, revoke).await;
            drop(client);
        };
        let (accepted, ()) = tokio::join!(inviter.serve_enrollment(server), disconnect);
        assert!(
            matches!(
                accepted,
                Err(EnrollmentError::Framing)
                    | Ok(EnrollmentOutcome::Unknown {
                        reason: EnrollmentError::Framing
                            | EnrollmentError::Authority(AuthorityError::Faulted),
                        ..
                    })
                    | Ok(EnrollmentOutcome::Rejected(EnrollmentRejection::Revoked))
            ),
            "revoke={revoke}"
        );
        assert!(!inviter.is_trusted(&joiner.local_pin().unwrap()));
        assert!(joiner.projection().unwrap().peers.is_empty());
        assert!(matches!(
            joiner.outbound_enrollments().unwrap()[0].state,
            OutboundEnrollmentState::Pending {}
        ));
    }
}

#[tokio::test]
async fn malformed_duplicate_and_over_capacity_enrollment_snapshots_fail_closed() {
    let (_inviter_directory, inviter_root, inviter) = persistent("inviter");
    let (_joiner_directory, joiner_root, joiner) = persistent("joiner");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    let (client, server) = connections(&inviter, &joiner).await;
    let (joined, accepted, _) = tokio::join!(
        joiner.redeem_enrollment(client, &invitation, transaction),
        inviter.serve_enrollment(server),
        approve(&inviter)
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
    assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
    drop((inviter, joiner));
    let inbound: serde_json::Value =
        serde_json::from_slice(&fs::read(inviter_root.join("state.json")).unwrap()).unwrap();
    let outbound: serde_json::Value =
        serde_json::from_slice(&fs::read(joiner_root.join("state.json")).unwrap()).unwrap();
    for case in [
        "receipt_duplicate",
        "receipt_pin",
        "receipt_id",
        "receipt_missing_peer",
        "receipt_revoked",
        "receipt_capacity",
        "receipt_name",
        "receipt_lifetime",
        "outbound_duplicate",
        "outbound_pin",
        "outbound_tx",
        "outbound_capacity",
        "outbound_lifetime",
        "outbound_revoked",
        "outbound_missing_receipt",
        "missing_field",
        "duplicate_field",
    ] {
        let is_outbound = case.starts_with("outbound");
        let root = if is_outbound {
            &joiner_root
        } else {
            &inviter_root
        };
        let mut value = if is_outbound {
            outbound.clone()
        } else {
            inbound.clone()
        };
        let receipt = value["receipts"][0].clone();
        let join = value["outbound"][0].clone();
        match case {
            "receipt_duplicate" => value["receipts"] = serde_json::json!([receipt, receipt]),
            "receipt_pin" => {
                value["receipts"][0]["pin"]["peer_id"] = value["identity"]["peer_id"].clone()
            }
            "receipt_id" => {
                value["receipts"][0]["receipt"]["inviter"] =
                    value["receipts"][0]["receipt"]["joiner"].clone()
            }
            "receipt_missing_peer" => value["peers"] = serde_json::json!([]),
            "receipt_revoked" => {
                value["tombstones"] = serde_json::json!([receipt["pin"]]);
                value["peers"] = serde_json::json!([]);
            }
            "receipt_capacity" => value["receipts"] = serde_json::json!(vec![receipt; 257]),
            "receipt_name" => {
                value["receipts"][0]["receipt"]["joiner_name"] = serde_json::json!("x".repeat(257))
            }
            "receipt_lifetime" => {
                value["receipts"][0]["receipt"]["inviter_lifetime"] = serde_json::json!("session")
            }
            "outbound_duplicate" => value["outbound"] = serde_json::json!([join, join]),
            "outbound_pin" => value["outbound"][0]["pin"]["spki"] = serde_json::json!([0]),
            "outbound_tx" => {
                value["outbound"][0]["transaction"] = serde_json::json!("AAAAAAAAAAAAAAAAAAAAAA")
            }
            "outbound_capacity" => value["outbound"] = serde_json::json!(vec![join; 257]),
            "outbound_lifetime" => {
                value["outbound"][0]["local_lifetime"] = serde_json::json!("session")
            }
            "outbound_revoked" => {
                value["tombstones"] = serde_json::json!([join["pin"]]);
                value["peers"] = serde_json::json!([]);
            }
            "outbound_missing_receipt" => {
                value["outbound"][0]["state"]["receipt"] = serde_json::Value::Null
            }
            "missing_field" => {
                value.as_object_mut().unwrap().remove("receipts");
            }
            "duplicate_field" => {}
            other => panic!("unknown case {other}"),
        }
        let mut bytes = serde_json::to_vec(&value).unwrap();
        if case == "duplicate_field" {
            bytes = format!(
                "{{\"receipts\":[],{}",
                &String::from_utf8(bytes).unwrap()[1..]
            )
            .into_bytes();
        }
        fs::write(root.join("state.json"), &bytes).unwrap();
        assert!(
            PeerAuthority::open_persistent(root, now()).is_err(),
            "{case}"
        );
        assert_eq!(fs::read(root.join("state.json")).unwrap(), bytes, "{case}");
    }
}
