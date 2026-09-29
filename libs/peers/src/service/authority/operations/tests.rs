use super::*;
use crate::{
    operations::{OperationBody, RequestId},
    session::SessionGeneration,
    StoreRevision,
};
use qol_conventions::{
    operations::{OperationKey, OperationKind},
    plugin_id::PluginUid,
};
use std::time::SystemTime;

fn key() -> OperationKey {
    OperationKey::new(
        PluginUid::new("fixture-uid"),
        OperationKind::Action,
        "count",
    )
}

fn body(arguments: &str) -> String {
    serde_json::to_string(&OperationBody {
        version: 1,
        key: key(),
        arguments: arguments.into(),
        timeout_ms: 10_000,
    })
    .unwrap()
}

fn link(authority: &PeerAuthority) -> AuthenticatedSession {
    let remote = crate::service::Identity::generate(SystemTime::now()).unwrap();
    authority
        .insert_link(
            authority.projection().unwrap().revision,
            remote.pin().clone(),
            "remote".into(),
        )
        .unwrap();
    let session = AuthenticatedSession {
        local_peer: authority.local_pin().unwrap().peer_id(),
        remote_peer: remote.pin().peer_id(),
        generation: SessionGeneration {
            local: random_nonce().unwrap(),
            remote: random_nonce().unwrap(),
        },
    };
    authority.elect_operation_session(session).unwrap();
    session
}

fn grant(authority: &PeerAuthority, session: AuthenticatedSession) {
    authority
        .set_grants(
            authority.projection().unwrap().revision,
            session.remote_peer,
            vec![key()],
        )
        .unwrap();
}

fn invocation(
    authority: &PeerAuthority,
    session: AuthenticatedSession,
    sequence: u64,
) -> Invocation {
    let body = body("{}");
    Invocation {
        handle: RequestHandle {
            recipient: session.local_peer,
            epoch: authority.operation_epoch(session.remote_peer).unwrap(),
            sequence: StoreRevision::new(sequence),
            id: RequestId(random_nonce().unwrap()),
            body_digest: body_digest(&body),
        },
        body,
    }
}

#[test]
fn operation_body_profile_rejects_lossy_numbers_duplicates_and_nonobjects() {
    for (arguments, valid) in [
        ("null", true),
        ("{}", true),
        (r#"{"n":9007199254740991,"nested":[true,null,"x"]}"#, true),
        (r#"{"n":-9007199254740991}"#, true),
        (r#"{"n":9007199254740992}"#, false),
        (r#"{"n":-0}"#, false),
        (r#"{"n":1.0}"#, false),
        (r#"{"n":1e1}"#, false),
        (r#"{"x":1,"\u0078":2}"#, false),
        ("[]", false),
        ("true", false),
    ] {
        assert_eq!(decode_body(&body(arguments)).is_ok(), valid, "{arguments}");
    }
    let first = body("{}");
    assert_ne!(body_digest(&first), body_digest(&(first.clone() + " ")));
    assert_ne!(body_digest(&body("{}")), body_digest(&body("{ }")));
    assert!(decode_body(&body(&format!("{{\"s\":\"{}\"}}", "x".repeat(2048)))).is_err());
}

#[test]
fn operation_ledger_duplicates_conflicts_gaps_cancellation_and_pruning_never_readmit() {
    let authority = PeerAuthority::session("local".into(), SystemTime::now()).unwrap();
    let session = link(&authority);
    let first = invocation(&authority, session, 1);
    assert_eq!(
        authority.admit_operation(session, &first, "d".repeat(43)),
        Err(Failure::GrantRequired)
    );
    grant(&authority, session);
    assert_eq!(
        authority.admit_operation(session, &first, "d".repeat(43)),
        Ok((Outcome::Accepted, true))
    );
    assert_eq!(
        authority.admit_operation(session, &first, "d".repeat(43)),
        Ok((Outcome::Accepted, false))
    );
    let mut conflict = first.clone();
    conflict.handle.id = RequestId(random_nonce().unwrap());
    assert_eq!(
        authority.admit_operation(session, &conflict, "d".repeat(43)),
        Err(Failure::Conflict)
    );
    assert_eq!(
        authority.start_operation(session, &first, &"e".repeat(43)),
        Err(Failure::ChangedDeclaration)
    );
    authority
        .start_operation(session, &first, &"d".repeat(43))
        .unwrap();
    assert_eq!(
        authority.cancel_operation(session, &first.handle),
        Ok(Outcome::DispatchStarted)
    );
    assert_eq!(
        authority.start_operation(session, &first, &"d".repeat(43)),
        Err(Failure::Busy)
    );
    authority
        .settle_operation(session.remote_peer, &first.handle, Outcome::Acknowledged)
        .unwrap();
    let gap = invocation(&authority, session, 3);
    assert_eq!(
        authority.admit_operation(session, &gap, "d".repeat(43)),
        Err(Failure::Gap)
    );
    let cancelled = invocation(&authority, session, 2);
    assert_eq!(
        authority.cancel_operation(session, &cancelled.handle),
        Ok(Outcome::CancelledBeforeDispatch)
    );
    assert_eq!(
        authority.admit_operation(session, &cancelled, "d".repeat(43)),
        Ok((Outcome::CancelledBeforeDispatch, false))
    );
    for sequence in 3..=20 {
        let request = invocation(&authority, session, sequence);
        authority
            .admit_operation(session, &request, "d".repeat(43))
            .unwrap();
        authority
            .cancel_operation(session, &request.handle)
            .unwrap();
    }
    assert_eq!(
        authority.operation_outcome(session, &first.handle),
        Err(Failure::Expired)
    );
    assert_eq!(
        authority.admit_operation(session, &first, "d".repeat(43)),
        Err(Failure::Expired)
    );
    assert_eq!(
        authority.lock().unwrap().state.operations[0].records.len(),
        16
    );
}

#[test]
fn operation_start_guard_rechecks_grants_session_and_deadline() {
    for reason in [
        Failure::GrantRequired,
        Failure::StaleSession,
        Failure::Deadline,
    ] {
        let authority = PeerAuthority::session("local".into(), SystemTime::now()).unwrap();
        let session = link(&authority);
        grant(&authority, session);
        let request = invocation(&authority, session, 1);
        authority
            .admit_operation(session, &request, "d".repeat(43))
            .unwrap();
        match reason {
            Failure::GrantRequired => {
                authority
                    .set_grants(
                        authority.projection().unwrap().revision,
                        session.remote_peer,
                        vec![],
                    )
                    .unwrap();
            }
            Failure::StaleSession => authority.close_operation_session(session),
            Failure::Deadline => {
                authority
                    .lock()
                    .unwrap()
                    .operation_deadlines
                    .get_mut(&session.remote_peer)
                    .unwrap()
                    .1 = Instant::now();
            }
            _ => unreachable!(),
        }
        assert_eq!(
            authority.start_operation(session, &request, &"d".repeat(43)),
            Err(reason)
        );
        authority
            .settle_operation(
                session.remote_peer,
                &request.handle,
                Outcome::CancelledBeforeDispatch,
            )
            .unwrap();
    }
}

#[test]
fn operation_revoke_after_start_settles_without_restoring_trust() {
    let authority = PeerAuthority::session("local".into(), SystemTime::now()).unwrap();
    let session = link(&authority);
    grant(&authority, session);
    let request = invocation(&authority, session, 1);
    authority
        .admit_operation(session, &request, "d".repeat(43))
        .unwrap();
    authority
        .start_operation(session, &request, &"d".repeat(43))
        .unwrap();
    authority
        .revoke(
            authority.projection().unwrap().revision,
            session.remote_peer,
        )
        .unwrap();
    assert_eq!(
        authority.operation_outcome(session, &request.handle),
        Err(Failure::Untrusted)
    );
    authority
        .settle_operation(session.remote_peer, &request.handle, Outcome::Unknown)
        .unwrap();
    assert!(authority.projection().unwrap().peers.is_empty());
    assert!(authority.lock().unwrap().state.operations.is_empty());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn operation_restart_commits_recovery_and_preserves_original_sender_allocation() {
    for started in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("authority");
        let authority =
            PeerAuthority::create_persistent(&root, "local".into(), SystemTime::now()).unwrap();
        let session = link(&authority);
        grant(&authority, session);
        authority
            .observe_operation_epoch(session, OperationEpoch(random_nonce().unwrap()))
            .unwrap();
        let outbound = authority
            .prepare_operation(session.remote_peer, body("{}"))
            .unwrap();
        let inbound = invocation(&authority, session, 1);
        authority
            .admit_operation(session, &inbound, "d".repeat(43))
            .unwrap();
        if started {
            authority
                .start_operation(session, &inbound, &"d".repeat(43))
                .unwrap();
        }
        drop(authority);
        let reopened = PeerAuthority::open_persistent(&root, SystemTime::now()).unwrap();
        reopened.elect_operation_session(session).unwrap();
        let expected = if started {
            Outcome::Unknown
        } else {
            Outcome::CancelledBeforeDispatch
        };
        assert_eq!(
            reopened.operation_outcome(session, &inbound.handle),
            Ok(expected)
        );
        assert_eq!(
            reopened.operation_requests(session.remote_peer).unwrap()[0].handle,
            outbound.handle
        );
        assert_eq!(
            reopened
                .prepare_operation(session.remote_peer, body("{}"))
                .err(),
            Some(Failure::Busy)
        );
        let bytes = std::fs::read(root.join("state.json")).unwrap();
        assert!(!String::from_utf8(bytes)
            .unwrap()
            .contains("dispatch_started"));
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn operation_v3_migration_preserves_authority_and_initializes_link_epochs_once() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("authority");
    let authority =
        PeerAuthority::create_persistent(&root, "local".into(), SystemTime::now()).unwrap();
    let session = link(&authority);
    grant(&authority, session);
    let original = authority.projection().unwrap();
    drop(authority);
    let path = root.join("state.json");
    let mut stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    stored["version"] = serde_json::json!(3);
    stored.as_object_mut().unwrap().remove("operations");
    std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    let authority = PeerAuthority::open_persistent(&root, SystemTime::now()).unwrap();
    let epoch = authority.operation_epoch(session.remote_peer).unwrap();
    assert_eq!(authority.projection().unwrap().peers, original.peers);
    assert_eq!(authority.local_pin().unwrap().peer_id(), original.peer_id);
    drop(authority);
    let authority = PeerAuthority::open_persistent(&root, SystemTime::now()).unwrap();
    assert_eq!(
        authority.operation_epoch(session.remote_peer).unwrap(),
        epoch
    );
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(stored["version"], 5);
}

#[test]
fn operation_concurrent_matching_admission_has_one_dispatch_owner() {
    let authority = PeerAuthority::session("local".into(), SystemTime::now()).unwrap();
    let session = link(&authority);
    grant(&authority, session);
    let request = invocation(&authority, session, 1);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let owners = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let barrier = barrier.clone();
                let authority = &authority;
                let request = &request;
                scope.spawn(move || {
                    barrier.wait();
                    authority
                        .admit_operation(session, request, "d".repeat(43))
                        .unwrap()
                        .1
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| usize::from(worker.join().unwrap()))
            .sum::<usize>()
    });
    assert_eq!(owners, 1);
}

#[test]
fn operation_old_epoch_overflow_and_busy_refuse_without_advancing_watermark() {
    let authority = PeerAuthority::session("local".into(), SystemTime::now()).unwrap();
    let session = link(&authority);
    grant(&authority, session);
    let mut request = invocation(&authority, session, 1);
    let original_epoch = request.handle.epoch;
    request.handle.epoch = OperationEpoch(random_nonce().unwrap());
    assert_eq!(
        authority.admit_operation(session, &request, "d".repeat(43)),
        Err(Failure::OldEpoch)
    );
    request.handle.epoch = original_epoch;
    authority
        .admit_operation(session, &request, "d".repeat(43))
        .unwrap();
    let next = invocation(&authority, session, 2);
    assert_eq!(
        authority.admit_operation(session, &next, "d".repeat(43)),
        Err(Failure::Busy)
    );
    assert_eq!(
        authority.lock().unwrap().state.operations[0]
            .admitted
            .value(),
        1
    );
    assert_eq!(
        authority.cancel_operation(session, &next.handle),
        Ok(Outcome::CancelledBeforeDispatch)
    );
    authority
        .observe_operation_epoch(session, OperationEpoch(random_nonce().unwrap()))
        .unwrap();
    authority.lock().unwrap().state.operations[0].next = StoreRevision::new(u64::MAX);
    assert_eq!(
        authority
            .prepare_operation(session.remote_peer, body("{}"))
            .err(),
        Some(Failure::Exhausted)
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn operation_dispatch_commit_fault_never_grants_start_and_writer_remains_owned() {
    use crate::service::authority::storage::CommitFault;
    for fault in [CommitFault::BeforeReplace, CommitFault::AfterReplace] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("authority");
        let authority =
            PeerAuthority::create_persistent(&root, "local".into(), SystemTime::now()).unwrap();
        let session = link(&authority);
        grant(&authority, session);
        let request = invocation(&authority, session, 1);
        authority
            .admit_operation(session, &request, "d".repeat(43))
            .unwrap();
        authority.lock().unwrap().storage.fail_next(fault);
        assert_eq!(
            authority.start_operation(session, &request, &"d".repeat(43)),
            Err(Failure::Storage)
        );
        assert_eq!(
            PeerAuthority::open_persistent(&root, SystemTime::now()).err(),
            Some(AuthorityError::WriterBusy)
        );
        drop(authority);
        let authority = PeerAuthority::open_persistent(&root, SystemTime::now()).unwrap();
        authority.elect_operation_session(session).unwrap();
        let recovered = authority
            .operation_outcome(session, &request.handle)
            .unwrap();
        assert!(matches!(
            recovered,
            Outcome::CancelledBeforeDispatch | Outcome::Unknown
        ));
        assert_eq!(
            authority.admit_operation(session, &request, "d".repeat(43)),
            Ok((recovered, false))
        );
    }
}

#[test]
fn operation_cancel_next_keeps_active_deadline_and_terminal_capacity_reserved() {
    let authority = PeerAuthority::session("local".into(), SystemTime::now()).unwrap();
    let session = link(&authority);
    grant(&authority, session);
    let active = invocation(&authority, session, 1);
    authority
        .admit_operation(session, &active, "d".repeat(43))
        .unwrap();
    let next = invocation(&authority, session, 2);
    authority.cancel_operation(session, &next.handle).unwrap();
    authority
        .start_operation(session, &active, &"d".repeat(43))
        .unwrap();
    let document = serde_json::to_string(&"\u{0001}".repeat(680)).unwrap();
    assert!(document.len() <= crate::operations::MAX_RESULT_BYTES);
    let outcome = authority
        .settle_operation(
            session.remote_peer,
            &active.handle,
            Outcome::Result { document },
        )
        .unwrap();
    assert!(outcome.terminal());
    let third = invocation(&authority, session, 3);
    assert_eq!(
        authority.admit_operation(session, &third, "d".repeat(43)),
        Ok((Outcome::Accepted, true))
    );
}

#[test]
fn operation_encoded_reservations_cover_sender_receiver_and_cancel_metadata() {
    let authority = PeerAuthority::session("local".into(), SystemTime::now()).unwrap();
    let session = link(&authority);
    grant(&authority, session);
    authority
        .observe_operation_epoch(session, OperationEpoch(random_nonce().unwrap()))
        .unwrap();
    let inbound = invocation(&authority, session, 1);
    authority
        .admit_operation(session, &inbound, "d".repeat(43))
        .unwrap();
    authority
        .prepare_operation(session.remote_peer, body("{}"))
        .unwrap();
    let inner = authority.lock().unwrap();
    let state = &inner.state;
    let reserved = state.validate_operations().unwrap();
    let link = &state.operations[0];
    let expected = 8192 - serde_json::to_vec(&link.records[0]).unwrap().len() + 8192
        - serde_json::to_vec(link.pending.as_ref().unwrap())
            .unwrap()
            .len()
        + 1024
        - serde_json::to_vec(&link.cancelled).unwrap().len();
    assert_eq!(reserved, expected);
    let mut invalid = state.clone();
    invalid.operations[0].records[0].declaration = "x".repeat(8192);
    assert_eq!(invalid.validate_operations(), Err(AuthorityError::Capacity));
}

#[test]
fn operation_global_record_limit_prunes_terminal_details_but_never_active_owners() {
    let authority = PeerAuthority::session("local".into(), SystemTime::now()).unwrap();
    let sessions: Vec<_> = (0..33).map(|_| link(&authority)).collect();
    let mut state = authority.lock().unwrap().state.clone();
    for session in &sessions[..32] {
        let link = state.operation_link_mut(session.remote_peer).unwrap();
        link.admitted = StoreRevision::new(16);
        for sequence in 1..=16 {
            let request = invocation(&authority, *session, sequence);
            link.records.push(ReceiverRecord {
                handle: request.handle,
                declaration: "d".repeat(43),
                outcome: Outcome::Acknowledged,
            });
        }
    }
    assert_eq!(
        state
            .operations
            .iter()
            .map(|link| link.records.len())
            .sum::<usize>(),
        512
    );
    state.reserve_receiver(sessions[32].remote_peer).unwrap();
    assert_eq!(
        state
            .operations
            .iter()
            .map(|link| link.records.len())
            .sum::<usize>(),
        511
    );
    let victim = state
        .operations
        .iter()
        .find(|link| link.records.len() == 15)
        .unwrap();
    assert_eq!(victim.admitted.value(), 16);
    let handle = invocation(
        &authority,
        *sessions
            .iter()
            .find(|session| session.remote_peer == victim.peer)
            .unwrap(),
        1,
    )
    .handle;
    assert_eq!(victim.lookup(&handle), Err(Failure::Expired));
    let mut state = authority.lock().unwrap().state.clone();
    let request = invocation(&authority, sessions[0], 1);
    let link = state.operation_link_mut(sessions[0].remote_peer).unwrap();
    link.admitted = request.handle.sequence;
    link.records.push(ReceiverRecord {
        handle: request.handle,
        declaration: "d".repeat(43),
        outcome: Outcome::DispatchStarted,
    });
    assert_eq!(
        state.reserve_receiver(sessions[0].remote_peer),
        Err(Failure::Busy)
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn operation_receiver_restart_keeps_terminal_result_epoch_and_grantless_outcome_access() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("authority");
    let authority =
        PeerAuthority::create_persistent(&root, "local".into(), SystemTime::now()).unwrap();
    let session = link(&authority);
    grant(&authority, session);
    let request = invocation(&authority, session, 1);
    authority
        .admit_operation(session, &request, "d".repeat(43))
        .unwrap();
    authority
        .start_operation(session, &request, &"d".repeat(43))
        .unwrap();
    let result = Outcome::Result {
        document: "{\"count\":1}".into(),
    };
    authority
        .settle_operation(session.remote_peer, &request.handle, result.clone())
        .unwrap();
    authority
        .set_grants(
            authority.projection().unwrap().revision,
            session.remote_peer,
            vec![],
        )
        .unwrap();
    drop(authority);
    let authority = PeerAuthority::open_persistent(&root, SystemTime::now()).unwrap();
    authority.elect_operation_session(session).unwrap();
    assert_eq!(
        authority.operation_epoch(session.remote_peer).unwrap(),
        request.handle.epoch
    );
    assert_eq!(
        authority.operation_outcome(session, &request.handle),
        Ok(result.clone())
    );
    assert_eq!(
        authority.admit_operation(session, &request, "d".repeat(43)),
        Ok((result, false))
    );
}
