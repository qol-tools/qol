use std::time::{Duration, SystemTime};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use qol_peers::{
    enrollment::{EnrollmentRejection, OutboundEnrollmentState, PendingEnrollment, TransactionId},
    service::{
        enrollment::{EnrollmentError, EnrollmentOutcome, Invitation},
        PeerAuthority, PeerConnection, TrustPolicy,
    },
    AuthorityError, StoreRevision,
};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

fn authority(name: &str) -> PeerAuthority {
    PeerAuthority::session(name.into(), SystemTime::now() - Duration::from_secs(60)).unwrap()
}

fn revision(authority: &PeerAuthority) -> StoreRevision {
    authority.projection().unwrap().revision
}

async fn connections(
    inviter: &PeerAuthority,
    joiner: &PeerAuthority,
) -> (PeerConnection<DuplexStream>, PeerConnection<DuplexStream>) {
    let client = joiner
        .enrollment_client_config(inviter.local_pin().unwrap())
        .unwrap();
    let server = inviter.enrollment_server_config().unwrap();
    let (client_io, server_io) = tokio::io::duplex(16 * 1024);
    let (client, server) = tokio::join!(client.connect(client_io), server.accept(server_io));
    (client.unwrap(), server.unwrap())
}

async fn pending(authority: &PeerAuthority) -> PendingEnrollment {
    let mut changes = authority.watch_enrollment();
    loop {
        if let Some(request) = authority.pending_enrollments().unwrap().into_iter().next() {
            return request;
        }
        changes.changed().await.unwrap();
    }
}

async fn approve(authority: &PeerAuthority) {
    let request = pending(authority).await;
    authority
        .approve_enrollment(revision(authority), request.key)
        .unwrap();
}

async fn enrolled(inviter: &PeerAuthority, joiner: &PeerAuthority) -> TransactionId {
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(joiner), &invitation).unwrap();
    let (client, server) = connections(inviter, joiner).await;
    let (joined, accepted, ()) = tokio::join!(
        joiner.redeem_enrollment(client, &invitation, transaction),
        inviter.serve_enrollment(server),
        approve(inviter),
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
    assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
    transaction
}

fn invitation_value(invitation: &Invitation) -> Value {
    let exported = invitation.export().unwrap();
    let bytes = URL_SAFE_NO_PAD
        .decode(exported.expose().strip_prefix("qol-link:").unwrap())
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn encode_invitation_value(value: &Value) -> String {
    let fields = [
        "version",
        "invitation",
        "secret",
        "inviter",
        "endpoints",
        "lifetime",
    ];
    let mut entries: Vec<_> = fields
        .into_iter()
        .map(|key| format!("\"{key}\":{}", value[key]))
        .collect();
    if let Some(extra) = value.get("extra") {
        entries.push(format!("\"extra\":{extra}"));
    }
    format!(
        "qol-link:{}",
        URL_SAFE_NO_PAD.encode(format!("{{{}}}", entries.join(",")))
    )
}

fn redeem_value(invitation: &Invitation, transaction: TransactionId, name: &str) -> Value {
    json!({"version":1,"invitation":invitation.id(),"transaction":transaction,
        "operation":{"kind":"redeem","secret":invitation_value(invitation)["secret"],"name":name,"lifetime":"session"}})
}

async fn send_value(connection: &mut PeerConnection<DuplexStream>, value: &Value) {
    let bytes = serde_json::to_vec(value).unwrap();
    connection
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    connection.write_all(&bytes).await.unwrap();
    connection.flush().await.unwrap();
}

async fn receive_value(connection: &mut PeerConnection<DuplexStream>) -> Value {
    let length = connection.read_u32().await.unwrap();
    let mut bytes = vec![0; length as usize];
    connection.read_exact(&mut bytes).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn actual_tls_enrollment_commits_mutual_authority_trust_with_empty_grants() {
    let inviter = authority("inviter");
    let joiner = authority("joiner");
    let transaction = enrolled(&inviter, &joiner).await;
    assert!(inviter.is_trusted(&joiner.local_pin().unwrap()));
    assert!(joiner.is_trusted(&inviter.local_pin().unwrap()));
    for peer in [&inviter, &joiner] {
        let projection = peer.projection().unwrap();
        assert_eq!(projection.peers.len(), 1);
        assert!(projection.peers[0].grants.is_empty());
        assert!(!serde_json::to_string(&projection)
            .unwrap()
            .contains("secret"));
    }
    let before = (revision(&inviter), revision(&joiner));
    let (client, server) = connections(&inviter, &joiner).await;
    let (joined, accepted) = tokio::join!(
        joiner.recover_enrollment(client, transaction),
        inviter.serve_enrollment(server)
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
    assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
    assert_eq!(before, (revision(&inviter), revision(&joiner)));
}

#[tokio::test(start_paused = true)]
async fn approval_waits_for_an_event_beyond_frame_timeout_without_extending_expiry() {
    let inviter = authority("inviter");
    let joiner = authority("joiner");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    let (client, server) = connections(&inviter, &joiner).await;
    let local = async {
        let request = pending(&inviter).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
        inviter
            .approve_enrollment(revision(&inviter), request.key)
            .unwrap();
    };
    let (joined, accepted, ()) = tokio::join!(
        joiner.redeem_enrollment(client, &invitation, transaction),
        inviter.serve_enrollment(server),
        local
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
    assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
}

#[tokio::test(start_paused = true)]
async fn cancel_and_expiry_deny_approval_without_trust_mutation() {
    for expire in [false, true] {
        let inviter = authority("inviter");
        let joiner = authority("joiner");
        let invitation = inviter.create_invitation(vec![]).unwrap();
        let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
        let (client, server) = connections(&inviter, &joiner).await;
        let local = async {
            let request = pending(&inviter).await;
            if expire {
                tokio::time::advance(Duration::from_secs(121)).await;
            } else {
                inviter.cancel_invitation(invitation.id()).unwrap();
            }
            assert!(inviter
                .approve_enrollment(revision(&inviter), request.key)
                .is_err());
        };
        let (joined, accepted, ()) = tokio::join!(
            joiner.redeem_enrollment(client, &invitation, transaction),
            inviter.serve_enrollment(server),
            local
        );
        assert!(!matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
        assert!(matches!(
            accepted,
            Ok(EnrollmentOutcome::Rejected(
                EnrollmentRejection::Expired | EnrollmentRejection::Cancelled
            )) | Err(EnrollmentError::Framing)
                | Ok(EnrollmentOutcome::Unknown {
                    reason: EnrollmentError::Framing,
                    ..
                })
        ));
        assert!(inviter.projection().unwrap().peers.is_empty());
        assert!(joiner.projection().unwrap().peers.is_empty());
    }
}

#[tokio::test]
async fn stale_and_concurrent_approvals_preserve_one_revision_and_one_receipt() {
    let inviter = authority("inviter");
    let joiner = authority("joiner");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    let (client, server) = connections(&inviter, &joiner).await;
    let local = async {
        let request = pending(&inviter).await;
        let old = revision(&inviter);
        inviter.rename(old, "renamed".into()).unwrap();
        assert!(matches!(
            inviter.approve_enrollment(old, request.key),
            Err(EnrollmentError::Authority(
                AuthorityError::StaleRevision { .. }
            ))
        ));
        let expected = revision(&inviter);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let inviter = inviter.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    inviter.approve_enrollment(expected, request.key)
                })
            })
            .collect();
        let outcomes: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(revision(&inviter).value(), expected.value() + 1);
    };
    let (joined, accepted, ()) = tokio::join!(
        joiner.redeem_enrollment(client, &invitation, transaction),
        inviter.serve_enrollment(server),
        local
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
    assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
}

#[tokio::test]
async fn wrong_token_and_malformed_frame_never_reach_pending_or_trust() {
    for case in [
        "token",
        "empty_frame",
        "extra_field",
        "long_name",
        "duplicate_field",
    ] {
        let inviter = authority("inviter");
        let joiner = authority("joiner");
        let invitation = inviter.create_invitation(vec![]).unwrap();
        let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
        let (mut client, server) = connections(&inviter, &joiner).await;
        let mut value = redeem_value(&invitation, transaction, "joiner");
        match case {
            "token" => value["operation"]["secret"] = json!(URL_SAFE_NO_PAD.encode([0_u8; 32])),
            "extra_field" => value["identity"] = json!(joiner.local_pin().unwrap().peer_id()),
            "long_name" => value["operation"]["name"] = json!("x".repeat(257)),
            _ => {}
        }
        let malicious = async {
            if case == "empty_frame" {
                client.write_all(&[0; 4]).await.unwrap();
                client.flush().await.unwrap();
            } else if case == "duplicate_field" {
                let bytes = format!(
                    "{{\"version\":1,{}",
                    &serde_json::to_string(&value).unwrap()[1..]
                );
                client
                    .write_all(&(bytes.len() as u32).to_be_bytes())
                    .await
                    .unwrap();
                client.write_all(bytes.as_bytes()).await.unwrap();
                client.flush().await.unwrap();
            } else {
                send_value(&mut client, &value).await;
            }
            let mut bytes = Vec::new();
            let _ = client.read_to_end(&mut bytes).await;
        };
        let (accepted, ()) = tokio::join!(inviter.serve_enrollment(server), malicious);
        assert!(
            !matches!(accepted, Ok(EnrollmentOutcome::Completed(_))),
            "{case}"
        );
        assert!(inviter.pending_enrollments().unwrap().is_empty(), "{case}");
        assert!(inviter.projection().unwrap().peers.is_empty(), "{case}");
    }
}

#[tokio::test]
async fn reservation_binds_key_transaction_name_and_lifetime() {
    let inviter = authority("inviter");
    let joiner = authority("joiner");
    let stranger = authority("stranger");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    let (mut client, server) = connections(&inviter, &joiner).await;
    send_value(
        &mut client,
        &redeem_value(&invitation, transaction, "joiner"),
    )
    .await;
    let server_authority = inviter.clone();
    let task = tokio::spawn(async move { server_authority.serve_enrollment(server).await });
    let original = pending(&inviter).await;
    for case in ["key", "transaction", "name", "lifetime"] {
        let (mut retry, server) =
            connections(&inviter, if case == "key" { &stranger } else { &joiner }).await;
        let mut value = redeem_value(&invitation, transaction, "joiner");
        match case {
            "transaction" => value["transaction"] = json!(URL_SAFE_NO_PAD.encode([7_u8; 16])),
            "name" => value["operation"]["name"] = json!("changed"),
            "lifetime" => value["operation"]["lifetime"] = json!("persistent"),
            _ => {}
        }
        send_value(&mut retry, &value).await;
        let (accepted, reply) =
            tokio::join!(inviter.serve_enrollment(server), receive_value(&mut retry));
        assert_eq!(
            accepted.unwrap(),
            EnrollmentOutcome::Rejected(EnrollmentRejection::Conflict)
        );
        assert_eq!(reply["outcome"]["reason"], "conflict");
        assert_eq!(
            inviter.pending_enrollments().unwrap(),
            vec![original.clone()]
        );
    }
    inviter.cancel_invitation(invitation.id()).unwrap();
    assert_eq!(
        task.await.unwrap().unwrap(),
        EnrollmentOutcome::Rejected(EnrollmentRejection::Cancelled)
    );
    assert!(inviter.projection().unwrap().peers.is_empty());
}

#[tokio::test]
async fn lost_approval_reply_reconciles_original_transaction_without_token() {
    let inviter = authority("inviter");
    let joiner = authority("joiner");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    let (mut client, server) = connections(&inviter, &joiner).await;
    send_value(
        &mut client,
        &redeem_value(&invitation, transaction, "joiner"),
    )
    .await;
    let local = async {
        let request = pending(&inviter).await;
        inviter
            .approve_enrollment(revision(&inviter), request.key)
            .unwrap();
        drop(client);
    };
    let (accepted, ()) = tokio::join!(inviter.serve_enrollment(server), local);
    assert!(matches!(
        accepted.unwrap(),
        EnrollmentOutcome::Unknown { .. }
    ));
    assert!(joiner.projection().unwrap().peers.is_empty());
    assert_eq!(inviter.projection().unwrap().peers.len(), 1);
    drop(invitation);
    let (client, server) = connections(&inviter, &joiner).await;
    let (joined, accepted) = tokio::join!(
        joiner.recover_enrollment(client, transaction),
        inviter.serve_enrollment(server)
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
    assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
}

#[tokio::test]
async fn revoked_receipt_cannot_recover_or_reinsert_a_link() {
    let inviter = authority("inviter");
    let joiner = authority("joiner");
    let transaction = enrolled(&inviter, &joiner).await;
    let (client, server) = connections(&inviter, &joiner).await;
    inviter
        .revoke(revision(&inviter), joiner.local_pin().unwrap().peer_id())
        .unwrap();
    let (joined, accepted) = tokio::join!(
        joiner.recover_enrollment(client, transaction),
        inviter.serve_enrollment(server)
    );
    assert_eq!(
        joined.unwrap(),
        EnrollmentOutcome::Rejected(EnrollmentRejection::Revoked)
    );
    assert_eq!(
        accepted.unwrap(),
        EnrollmentOutcome::Rejected(EnrollmentRejection::Revoked)
    );
    assert!(inviter.projection().unwrap().peers.is_empty());
    assert!(!inviter.is_trusted(&joiner.local_pin().unwrap()));
}

#[tokio::test]
async fn wrong_inviter_pin_fails_tls_before_any_enrollment() {
    let inviter = authority("inviter");
    let joiner = authority("joiner");
    let stranger = authority("stranger");
    let client = joiner
        .enrollment_client_config(stranger.local_pin().unwrap())
        .unwrap();
    let server = inviter.enrollment_server_config().unwrap();
    let (client_io, server_io) = tokio::io::duplex(16 * 1024);
    let (client, server) = tokio::join!(client.connect(client_io), server.accept(server_io));
    assert!(client.is_err());
    assert!(server.is_err());
    assert!(inviter.pending_enrollments().unwrap().is_empty());
}

#[test]
fn invitation_import_is_canonical_bounded_redacted_and_strict() {
    let inviter = authority("inviter");
    let invitation = inviter
        .create_invitation(vec!["127.0.0.1:4567".parse().unwrap()])
        .unwrap();
    let exported = invitation.export().unwrap();
    let imported = Invitation::import(exported.expose()).unwrap();
    assert_eq!(imported.id(), invitation.id());
    assert_eq!(imported.inviter_pin(), invitation.inviter_pin());
    assert!(imported.export().unwrap().expose() == exported.expose());
    assert!(Invitation::import(&encode_invitation_value(&invitation_value(&invitation))).is_ok());
    assert_eq!(format!("{invitation:?}"), "Invitation([REDACTED])");
    assert_eq!(format!("{exported:?}"), "ExportedInvitation([REDACTED])");
    let body = exported.expose().strip_prefix("qol-link:").unwrap();
    let bytes = URL_SAFE_NO_PAD.decode(body).unwrap();
    let raw = String::from_utf8(bytes).unwrap();
    assert!(raw.starts_with("{\"version\":1,\"invitation\":"));
    for invalid in [
        format!("{}=", exported.expose()),
        format!("qol-link:{}", URL_SAFE_NO_PAD.encode(format!(" {raw}"))),
        format!(
            "qol-link:{}",
            URL_SAFE_NO_PAD.encode(format!("{{\"version\":1,{}", &raw[1..]))
        ),
        "qol-link:".to_string(),
        format!("qol-link:{}", "x".repeat(4096)),
    ] {
        assert_eq!(
            Invitation::import(&invalid).unwrap_err(),
            EnrollmentError::InvalidInvitation
        );
    }
    for case in [
        "version",
        "zero_port",
        "hostname",
        "pin",
        "secret",
        "unknown",
        "endpoints",
    ] {
        let mut value = invitation_value(&invitation);
        match case {
            "version" => value["version"] = json!(2),
            "zero_port" => value["endpoints"] = json!(["127.0.0.1:0"]),
            "hostname" => value["endpoints"] = json!(["example.invalid:4567"]),
            "pin" => value["inviter"] = json!("invalid"),
            "secret" => value["secret"] = json!("invalid"),
            "unknown" => value["extra"] = json!(true),
            "endpoints" => value["endpoints"] = json!(vec!["127.0.0.1:4567"; 9]),
            _ => unreachable!(),
        }
        let encoded = encode_invitation_value(&value);
        assert!(Invitation::import(&encoded).is_err(), "{case}");
    }
}

#[tokio::test(start_paused = true)]
async fn invitation_and_outbound_caps_fail_before_mutation() {
    let inviter = authority("inviter");
    for _ in 0..8 {
        inviter.create_invitation(vec![]).unwrap();
    }
    assert_eq!(
        inviter.create_invitation(vec![]).unwrap_err(),
        EnrollmentError::Rejected(EnrollmentRejection::Capacity)
    );
    tokio::time::advance(Duration::from_secs(121)).await;
    inviter.create_invitation(vec![]).unwrap();
    let joiner = authority("joiner");
    for _ in 0..32 {
        let remote = authority("remote");
        joiner
            .prepare_join(
                revision(&joiner),
                &remote.create_invitation(vec![]).unwrap(),
            )
            .unwrap();
    }
    let before = revision(&joiner);
    let remote = authority("remote");
    assert_eq!(
        joiner.prepare_join(before, &remote.create_invitation(vec![]).unwrap()),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Capacity))
    );
    assert_eq!(revision(&joiner), before);
    assert_eq!(joiner.outbound_enrollments().unwrap().len(), 32);
}

#[cfg(target_os = "linux")]
mod persistent {
    use super::*;

    #[tokio::test]
    async fn restart_retains_pending_intent_and_consumed_receipt_without_invitation_secret() {
        let temporary = tempfile::tempdir().unwrap();
        let inviter_root = temporary.path().join("inviter");
        let joiner_root = temporary.path().join("joiner");
        let now = SystemTime::now() - Duration::from_secs(60);
        let inviter =
            PeerAuthority::create_persistent(&inviter_root, "inviter".into(), now).unwrap();
        let joiner = PeerAuthority::create_persistent(&joiner_root, "joiner".into(), now).unwrap();
        let invitation = inviter.create_invitation(vec![]).unwrap();
        let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
        let (mut client, server) = connections(&inviter, &joiner).await;
        let mut value = redeem_value(&invitation, transaction, "joiner");
        value["operation"]["lifetime"] = json!("persistent");
        send_value(&mut client, &value).await;
        let local = async {
            approve(&inviter).await;
            drop(client);
        };
        let (result, ()) = tokio::join!(inviter.serve_enrollment(server), local);
        assert!(matches!(result.unwrap(), EnrollmentOutcome::Unknown { .. }));
        let token = invitation_value(&invitation)["secret"]
            .as_str()
            .unwrap()
            .to_owned();
        for root in [&inviter_root, &joiner_root] {
            let snapshot = std::fs::read_to_string(root.join("state.json")).unwrap();
            assert!(!snapshot.contains(&token));
            assert!(!snapshot.contains("\"secret\""));
            assert_eq!(
                serde_json::from_str::<Value>(&snapshot).unwrap()["version"],
                4
            );
        }
        drop((inviter, joiner, invitation));
        let inviter = PeerAuthority::open_persistent(&inviter_root, now).unwrap();
        let joiner = PeerAuthority::open_persistent(&joiner_root, now).unwrap();
        assert_eq!(
            joiner.outbound_enrollments().unwrap()[0].key.transaction,
            transaction
        );
        let (client, server) = connections(&inviter, &joiner).await;
        let (joined, accepted) = tokio::join!(
            joiner.recover_enrollment(client, transaction),
            inviter.serve_enrollment(server)
        );
        assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
        assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
        assert!(inviter.is_trusted(&joiner.local_pin().unwrap()));
        assert!(joiner.is_trusted(&inviter.local_pin().unwrap()));
    }
}

#[tokio::test]
async fn completed_outbound_evidence_does_not_consume_pending_capacity() {
    let joiner = authority("joiner");
    let inviter = authority("inviter");
    enrolled(&inviter, &joiner).await;
    for _ in 0..32 {
        let remote = authority("remote");
        let invitation = remote.create_invitation(vec![]).unwrap();
        joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    }
    let records = joiner.outbound_enrollments().unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|entry| matches!(entry.state, OutboundEnrollmentState::Pending {}))
            .count(),
        32
    );
    assert_eq!(
        records
            .iter()
            .filter(|entry| matches!(entry.state, OutboundEnrollmentState::Committed { .. }))
            .count(),
        1
    );
    let remote = authority("overflow");
    let invitation = remote.create_invitation(vec![]).unwrap();
    assert_eq!(
        joiner.prepare_join(revision(&joiner), &invitation),
        Err(EnrollmentError::Rejected(EnrollmentRejection::Capacity))
    );
}
