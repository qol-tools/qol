mod expiry;
mod lifecycle;
#[cfg(target_os = "linux")]
mod persistence;
mod schema;

use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::enrollment::{
    EnrollmentReceipt, EnrollmentVersion, OutboundEnrollmentState, TransactionId,
};
use crate::service::{
    enrollment::{
        wire::{Request, RequestOperation, Response, ResponseOutcome},
        EnrollmentOutcome, Invitation,
    },
    framing::{read_json, write_json, FrameLimit},
    Identity, PeerAuthority, PeerConnection, TrustPolicy,
};
use crate::{AuthorityLifetime, StoreRevision};
use tokio::io::DuplexStream;

fn now() -> SystemTime {
    SystemTime::now() - Duration::from_secs(60)
}

fn session(name: &str) -> PeerAuthority {
    PeerAuthority::session(name.into(), now()).unwrap()
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

async fn approve(authority: &PeerAuthority) -> EnrollmentReceipt {
    let mut events = authority.watch_enrollment();
    loop {
        if let Some(request) = authority.pending_enrollments().unwrap().first() {
            return authority
                .approve_enrollment(revision(authority), request.key)
                .unwrap();
        }
        events.changed().await.unwrap();
    }
}

fn request(
    invitation: &Invitation,
    transaction: TransactionId,
    lifetime: AuthorityLifetime,
) -> Request {
    Request {
        version: EnrollmentVersion::V1,
        invitation: invitation.id(),
        transaction,
        operation: RequestOperation::Redeem {
            secret: invitation.document.secret.clone(),
            name: "joiner".into(),
            lifetime,
        },
    }
}

#[test]
fn frozen_wire_fixtures_reject_unknown_tags_versions_and_changed_shapes() {
    let authority = session("inviter");
    let invitation = authority.create_invitation(vec![]).unwrap();
    let transaction: TransactionId = "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap();
    let redemption = request(&invitation, transaction, AuthorityLifetime::Session);
    let redemption = serde_json::to_vec(&redemption).unwrap();
    assert!(serde_json::from_slice::<Request>(&redemption).is_ok());
    let request = Request {
        version: EnrollmentVersion::V1,
        invitation: invitation.id(),
        transaction,
        operation: RequestOperation::Recover {},
    };
    assert_eq!(serde_json::to_string(&request).unwrap(), format!("{{\"version\":1,\"invitation\":\"{}\",\"transaction\":\"AAAAAAAAAAAAAAAAAAAAAA\",\"operation\":{{\"kind\":\"recover\"}}}}", invitation.id()));
    let fixture = serde_json::to_value(&request).unwrap();
    let pending = Response {
        version: EnrollmentVersion::V1,
        invitation: invitation.id(),
        transaction,
        outcome: ResponseOutcome::Pending {},
    };
    assert_eq!(
        serde_json::to_value(&pending).unwrap(),
        serde_json::json!({
            "version": 1, "invitation": invitation.id(), "transaction": "AAAAAAAAAAAAAAAAAAAAAA", "outcome": {"kind":"pending"}
        })
    );
    for case in [
        "version",
        "unknown",
        "tag",
        "secret_on_recover",
        "transaction",
        "duplicate",
    ] {
        let mut value = fixture.clone();
        match case {
            "version" => value["version"] = serde_json::json!("1"),
            "unknown" => value["extra"] = serde_json::json!(true),
            "tag" => value["operation"]["kind"] = serde_json::json!("invoke"),
            "secret_on_recover" => value["operation"]["secret"] = serde_json::json!("hidden"),
            "transaction" => value["transaction"] = serde_json::json!("AAAAAAAAAAAAAAAAAAAAAA="),
            "duplicate" => {
                let raw = serde_json::to_string(&value).unwrap();
                assert!(
                    serde_json::from_str::<Request>(&format!("{{\"version\":1,{}", &raw[1..]))
                        .is_err()
                );
                continue;
            }
            other => panic!("unknown case {other}"),
        }
        assert!(serde_json::from_value::<Request>(value).is_err(), "{case}");
    }
}

#[tokio::test]
async fn absent_or_false_client_possession_cannot_create_an_authority_reservation() {
    use rustls::{
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName},
        sign::{CertifiedKey, SingleCertAndKey},
    };
    for malicious in [false, true] {
        let inviter = session("inviter");
        let identity = inviter.inner.lock().unwrap().state.identity.clone();
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(CertificateDer::from(identity.certificate_der().to_vec()))
            .unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(roots);
        let mut config = if malicious {
            let claimed = Identity::generate(now()).unwrap();
            let other = Identity::generate(now()).unwrap();
            let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                other.export_secret().expose_pkcs8().to_vec(),
            ));
            let signing = provider.key_provider.load_private_key(key).unwrap();
            let certified = CertifiedKey::new(
                vec![CertificateDer::from(claimed.certificate_der().to_vec())],
                signing,
            );
            builder.with_client_cert_resolver(Arc::new(SingleCertAndKey::from(Arc::new(certified))))
        } else {
            builder.with_no_client_auth()
        };
        config.alpn_protocols = vec![crate::service::ENROLLMENT_ALPN.to_vec()];
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let server = inviter.enrollment_server_config().unwrap();
        let (client_io, server_io) = tokio::io::duplex(16 * 1024);
        let (_, result) = tokio::join!(
            connector.connect(ServerName::try_from("qol-peer.invalid").unwrap(), client_io),
            server.accept(server_io)
        );
        if malicious {
            assert!(matches!(
                result,
                Err(crate::service::PeerError::HandshakeSignature)
            ));
        } else {
            assert!(matches!(
                result,
                Err(crate::service::PeerError::MissingIdentity)
            ));
        }
        assert!(inviter.pending_enrollments().unwrap().is_empty());
        assert!(inviter.projection().unwrap().peers.is_empty());
    }
}

#[tokio::test]
async fn simultaneous_duplicate_redemptions_share_one_commit() {
    let inviter = session("inviter");
    let joiner = session("joiner");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    let (first_client, first_server) = connections(&inviter, &joiner).await;
    let (second_client, second_server) = connections(&inviter, &joiner).await;
    let (one, two, server_one, server_two, _) = tokio::join!(
        joiner.redeem_enrollment(first_client, &invitation, transaction),
        joiner.redeem_enrollment(second_client, &invitation, transaction),
        inviter.serve_enrollment(first_server),
        inviter.serve_enrollment(second_server),
        approve(&inviter),
    );
    for outcome in [one, two, server_one, server_two] {
        assert!(matches!(outcome.unwrap(), EnrollmentOutcome::Completed(_)));
    }
    assert_eq!(revision(&inviter).value(), 1);
    assert_eq!(revision(&joiner).value(), 2);
    assert_eq!(inviter.inner.lock().unwrap().state.receipts.len(), 1);
    assert!(inviter.is_trusted(&joiner.local_pin().unwrap()));
}

#[tokio::test]
async fn unexpected_response_and_receipt_substitution_leave_original_pending_intent() {
    for case in [
        "outer_transaction",
        "inner_transaction",
        "inviter",
        "joiner",
        "invitation",
        "name",
        "lifetime",
        "premature_confirmation",
    ] {
        let inviter = session("inviter");
        let joiner = session("joiner");
        let stranger = session("stranger");
        let invitation = inviter.create_invitation(vec![]).unwrap();
        let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
        let (client, mut server) = connections(&inviter, &joiner).await;
        let malicious = async {
            let _: Request = read_json(&mut server, FrameLimit::Enrollment)
                .await
                .unwrap();
            let mut receipt = EnrollmentReceipt {
                invitation: invitation.id(),
                transaction,
                inviter: inviter.local_pin().unwrap().peer_id(),
                joiner: joiner.local_pin().unwrap().peer_id(),
                inviter_name: "inviter".into(),
                joiner_name: "joiner".into(),
                inviter_lifetime: AuthorityLifetime::Session,
                joiner_lifetime: AuthorityLifetime::Session,
            };
            let wrong: TransactionId = "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap();
            match case {
                "inner_transaction" => receipt.transaction = wrong,
                "inviter" => receipt.inviter = stranger.local_pin().unwrap().peer_id(),
                "joiner" => receipt.joiner = stranger.local_pin().unwrap().peer_id(),
                "invitation" => receipt.invitation = "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap(),
                "name" => receipt.joiner_name = "changed".into(),
                "lifetime" => receipt.inviter_lifetime = AuthorityLifetime::Persistent,
                "outer_transaction" | "premature_confirmation" => {}
                other => panic!("unknown case {other}"),
            }
            let response = Response {
                version: EnrollmentVersion::V1,
                invitation: invitation.id(),
                transaction: if case == "outer_transaction" {
                    wrong
                } else {
                    transaction
                },
                outcome: if case == "premature_confirmation" {
                    ResponseOutcome::Confirmed { receipt }
                } else {
                    ResponseOutcome::Committed { receipt }
                },
            };
            write_json(&mut server, &response, FrameLimit::Enrollment)
                .await
                .unwrap();
        };
        let (outcome, ()) = tokio::join!(
            joiner.redeem_enrollment(client, &invitation, transaction),
            malicious
        );
        assert!(
            matches!(outcome.unwrap(), EnrollmentOutcome::Unknown { .. }),
            "{case}"
        );
        assert!(joiner.projection().unwrap().peers.is_empty(), "{case}");
        assert_eq!(
            joiner.outbound_enrollments().unwrap()[0].key.transaction,
            transaction
        );
    }
}

#[tokio::test]
async fn normal_session_cannot_enter_enrollment_exchange() {
    let inviter = session("inviter");
    let joiner = session("joiner");
    inviter
        .insert_link(
            revision(&inviter),
            joiner.local_pin().unwrap(),
            "joiner".into(),
        )
        .unwrap();
    joiner
        .insert_link(
            revision(&joiner),
            inviter.local_pin().unwrap(),
            "inviter".into(),
        )
        .unwrap();
    let identity = joiner.inner.lock().unwrap().state.identity.clone();
    let client =
        crate::service::NormalClientConfig::new(&identity, inviter.local_pin().unwrap()).unwrap();
    let server = inviter.server_config().unwrap();
    let (client_io, server_io) = tokio::io::duplex(16 * 1024);
    let (client, server) = tokio::join!(client.connect(client_io), server.accept(server_io));
    assert!(client.is_ok());
    assert_eq!(
        inviter.serve_enrollment(server.unwrap()).await,
        Err(crate::service::enrollment::EnrollmentError::Protocol)
    );
}

#[tokio::test(start_paused = true)]
async fn only_matching_committed_receipt_survives_expiry() {
    let inviter = session("inviter");
    let joiner = session("joiner");
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
    tokio::time::advance(Duration::from_secs(121)).await;
    let (client, server) = connections(&inviter, &joiner).await;
    let (joined, accepted) = tokio::join!(
        joiner.recover_enrollment(client, transaction),
        inviter.serve_enrollment(server)
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
    assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
    let (mut client, server) = connections(&inviter, &joiner).await;
    let bad = Request {
        version: EnrollmentVersion::V1,
        invitation: invitation.id(),
        transaction: "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap(),
        operation: RequestOperation::Recover {},
    };
    write_json(&mut client, &bad, FrameLimit::Enrollment)
        .await
        .unwrap();
    let (accepted, response) = tokio::join!(
        inviter.serve_enrollment(server),
        read_json::<_, Response>(&mut client, FrameLimit::Enrollment)
    );
    assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Rejected(_)));
    assert!(matches!(
        response.unwrap().outcome,
        ResponseOutcome::Rejected { .. }
    ));
}

#[tokio::test]
async fn lost_final_reply_is_unknown_even_after_joiner_commit_and_confirmation_write() {
    use crate::service::enrollment::wire::Confirmation;
    let inviter = session("inviter");
    let joiner = session("joiner");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    let (client, mut server) = connections(&inviter, &joiner).await;
    let lost_reply = async {
        let request: Request = read_json(&mut server, FrameLimit::Enrollment)
            .await
            .unwrap();
        inviter
            .reserve_enrollment(server.remote_identity().pin(), &request)
            .unwrap();
        let receipt = approve(&inviter).await;
        let response = Response {
            version: EnrollmentVersion::V1,
            invitation: invitation.id(),
            transaction,
            outcome: ResponseOutcome::Committed {
                receipt: receipt.clone(),
            },
        };
        write_json(&mut server, &response, FrameLimit::Enrollment)
            .await
            .unwrap();
        let confirmation: Confirmation = read_json(&mut server, FrameLimit::Enrollment)
            .await
            .unwrap();
        assert_eq!(confirmation.receipt, receipt);
        drop(server);
    };
    let (joined, ()) = tokio::join!(
        joiner.redeem_enrollment(client, &invitation, transaction),
        lost_reply
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Unknown { .. }));
    assert!(matches!(
        joiner.outbound_enrollments().unwrap()[0].state,
        OutboundEnrollmentState::Committed { .. }
    ));
    assert!(joiner.is_trusted(&inviter.local_pin().unwrap()));
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

#[tokio::test]
async fn completed_enrollment_feeds_both_live_normal_tls_trust_policies() {
    let inviter = session("inviter");
    let joiner = session("joiner");
    let inviter_server = inviter.server_config().unwrap();
    let joiner_server = joiner.server_config().unwrap();
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
    for (source, target, server) in [
        (&joiner, &inviter, &inviter_server),
        (&inviter, &joiner, &joiner_server),
    ] {
        let identity = source.inner.lock().unwrap().state.identity.clone();
        let client =
            crate::service::NormalClientConfig::new(&identity, target.local_pin().unwrap())
                .unwrap();
        let (client_io, server_io) = tokio::io::duplex(16 * 1024);
        let (connected, accepted) =
            tokio::join!(client.connect(client_io), server.accept(server_io));
        assert!(connected.is_ok());
        assert_eq!(
            accepted.unwrap().remote_identity().pin(),
            &source.local_pin().unwrap()
        );
        assert!(target.projection().unwrap().peers[0].grants.is_empty());
    }
    inviter
        .revoke(revision(&inviter), joiner.local_pin().unwrap().peer_id())
        .unwrap();
    let identity = joiner.inner.lock().unwrap().state.identity.clone();
    let client =
        crate::service::NormalClientConfig::new(&identity, inviter.local_pin().unwrap()).unwrap();
    let (client_io, server_io) = tokio::io::duplex(16 * 1024);
    let (_, rejected) = tokio::join!(client.connect(client_io), inviter_server.accept(server_io));
    assert!(matches!(
        rejected,
        Err(crate::service::PeerError::UntrustedPeer)
    ));
}

#[tokio::test]
async fn unknown_recovery_cannot_reserve_an_unused_invitation() {
    let inviter = session("inviter");
    let joiner = session("joiner");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = joiner.prepare_join(revision(&joiner), &invitation).unwrap();
    let (client, server) = connections(&inviter, &joiner).await;
    let (joined, accepted) = tokio::join!(
        joiner.recover_enrollment(client, transaction),
        inviter.serve_enrollment(server)
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Unknown { .. }));
    assert!(matches!(
        accepted.unwrap(),
        EnrollmentOutcome::Unknown { .. }
    ));
    assert!(inviter.pending_enrollments().unwrap().is_empty());
    assert!(inviter.projection().unwrap().peers.is_empty());
    let (client, server) = connections(&inviter, &joiner).await;
    let (joined, accepted, _) = tokio::join!(
        joiner.redeem_enrollment(client, &invitation, transaction),
        inviter.serve_enrollment(server),
        approve(&inviter)
    );
    assert!(matches!(joined.unwrap(), EnrollmentOutcome::Completed(_)));
    assert!(matches!(accepted.unwrap(), EnrollmentOutcome::Completed(_)));
}
