use std::{
    future::pending,
    sync::atomic::Ordering,
    time::{Duration, SystemTime},
};

use tokio::{io::duplex, time::sleep};

use crate::{
    service::{
        framing::{read_json, write_json, FrameLimit},
        Identity, PeerAuthority,
    },
    session::SessionNonce,
    AuthorityError,
};

use super::{
    link, revoke, wire::fixtures, Fixture, Message, NormalSession, SessionError, SessionOutcome,
    Version,
};

#[tokio::test(start_paused = true)]
async fn public_constructors_bind_both_tls_identities_to_their_session_authorities() {
    let fixture = Fixture::new();
    let left_id = fixture.left.local_pin().unwrap().peer_id();
    let right_id = fixture.right.local_pin().unwrap().peer_id();
    let (left_io, right_io) = duplex(64);
    let (left, right) = tokio::join!(
        NormalSession::connect(left_io, fixture.left.clone(), right_id),
        NormalSession::accept(right_io, fixture.right.clone()),
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.authenticated().unwrap().local_peer, left_id);
    assert_eq!(left.authenticated().unwrap().remote_peer, right_id);
    assert_eq!(right.authenticated().unwrap().local_peer, right_id);
    assert_eq!(right.authenticated().unwrap().remote_peer, left_id);
    let (left, right) = tokio::join!(
        left.run(sleep(Duration::from_secs(1))),
        right.run(pending())
    );
    assert_eq!(left, SessionOutcome::Cancelled);
    assert_eq!(right, SessionOutcome::Closed(SessionError::Transport));
}

#[tokio::test(start_paused = true)]
async fn mutual_tls_bounded_hello_and_reconnect_have_fresh_directional_generations() {
    let fixture = Fixture::new();
    let mut previous = None;
    for round in 0..3 {
        let (left, right) = fixture.sessions(64).await;
        let a = left.authenticated().unwrap();
        let b = right.authenticated().unwrap();
        assert_eq!(a.local_peer, b.remote_peer, "round {round}");
        assert_eq!(a.remote_peer, b.local_peer, "round {round}");
        assert_eq!(a.generation.local, b.generation.remote, "round {round}");
        assert_eq!(a.generation.remote, b.generation.local, "round {round}");
        if let Some(old) = previous {
            assert_ne!(a.generation, old, "round {round}");
        }
        previous = Some(a.generation);
        drop((left, right));
        assert_eq!(fixture.drops.load(Ordering::SeqCst), (round + 1) * 2);
    }
}

#[test]
fn outbound_config_uses_only_current_authority_links() {
    let fixture = Fixture::new();
    let local = fixture.left.local_pin().unwrap().peer_id();
    let remote = fixture.right.local_pin().unwrap().peer_id();
    let unknown = Identity::generate(SystemTime::now())
        .unwrap()
        .pin()
        .peer_id();
    assert_eq!(
        fixture.left.normal_client_config(local).err(),
        Some(AuthorityError::LocalPeer)
    );
    assert_eq!(
        fixture.left.normal_client_config(unknown).err(),
        Some(AuthorityError::UnknownPeer)
    );
    assert!(fixture.left.normal_client_config(remote).is_ok());
    revoke(&fixture.left, &fixture.right);
    assert_eq!(
        fixture.left.normal_client_config(remote).err(),
        Some(AuthorityError::UnknownPeer)
    );
    fixture.left.fault_for_session_test();
    assert_eq!(
        fixture.left.normal_client_config(remote).err(),
        Some(AuthorityError::Faulted)
    );
}

#[tokio::test(start_paused = true)]
async fn admission_rechecks_revocation_fault_and_authority_after_tls() {
    for mode in ["revoked", "faulted", "wrong_authority"] {
        let fixture = Fixture::new();
        let (left, right) = fixture.connections(4096).await;
        let authority = match mode {
            "revoked" => {
                revoke(&fixture.left, &fixture.right);
                fixture.left.clone()
            }
            "faulted" => {
                fixture.left.fault_for_session_test();
                fixture.left.clone()
            }
            "wrong_authority" => PeerAuthority::session("other".into(), SystemTime::now()).unwrap(),
            _ => unreachable!(),
        };
        let expected = if mode == "faulted" {
            SessionError::AuthorityUnavailable
        } else {
            SessionError::Untrusted
        };
        assert_eq!(
            NormalSession::open(left, authority).await.err(),
            Some(expected),
            "{mode}"
        );
        drop(right);
        assert_eq!(fixture.drops.load(Ordering::SeqCst), 2, "{mode}");
    }
}

#[tokio::test(start_paused = true)]
async fn enrollment_mode_and_local_peer_are_rejected() {
    let fixture = Fixture::new();
    let client = fixture
        .left
        .enrollment_client_config(fixture.right.local_pin().unwrap())
        .unwrap();
    let server = fixture.right.enrollment_server_config().unwrap();
    let (left, right) = duplex(4096);
    let (left, right) = tokio::join!(client.connect(left), server.accept(right));
    assert_eq!(
        NormalSession::open(left.unwrap(), fixture.left.clone())
            .await
            .err(),
        Some(SessionError::WrongMode)
    );
    drop(right);
    let (left, right) = fixture.connections(4096).await;
    assert_eq!(
        NormalSession::open(left, fixture.right.clone()).await.err(),
        Some(SessionError::Untrusted)
    );
    drop(right);
}

#[tokio::test(start_paused = true)]
async fn hello_requires_both_tls_bound_ids_and_hello_tag() {
    for mode in ["sender", "recipient", "heartbeat"] {
        let fixture = Fixture::new();
        let (left, mut right) = fixture.connections(4096).await;
        let (_, hello) = fixtures();
        let (result, ()) = tokio::join!(NormalSession::open(left, fixture.left.clone()), async {
            let incoming: Message = read_json(&mut right, FrameLimit::Normal).await.unwrap();
            let Message::Hello {
                sender, recipient, ..
            } = incoming
            else {
                panic!("hello")
            };
            let outgoing = match mode {
                "sender" => Message::Hello {
                    version: Version,
                    sender,
                    recipient: sender,
                    nonce: SessionNonce::from_random([1; 16]),
                },
                "recipient" => Message::Hello {
                    version: Version,
                    sender: recipient,
                    recipient,
                    nonce: SessionNonce::from_random([1; 16]),
                },
                "heartbeat" => hello,
                _ => unreachable!(),
            };
            write_json(&mut right, &outgoing, FrameLimit::Normal)
                .await
                .unwrap();
        });
        assert_eq!(result.err(), Some(SessionError::Protocol), "{mode}");
    }
}

#[tokio::test(start_paused = true)]
async fn hello_watch_closes_on_revoke_or_fault_without_waiting_for_deadline() {
    for fault in [false, true] {
        let fixture = Fixture::new();
        let (left, right) = fixture.connections(4096).await;
        let (result, ()) = tokio::join!(NormalSession::open(left, fixture.left.clone()), async {
            sleep(Duration::from_secs(1)).await;
            if fault {
                fixture.left.fault_for_session_test();
                return;
            }
            revoke(&fixture.left, &fixture.right);
        });
        let expected = if fault {
            SessionError::AuthorityUnavailable
        } else {
            SessionError::Untrusted
        };
        assert_eq!(result.err(), Some(expected), "fault {fault}");
        drop(right);
    }
}

#[tokio::test(start_paused = true)]
async fn old_client_config_cannot_make_revoked_tls_into_a_session() {
    let fixture = Fixture::new();
    let config = fixture
        .left
        .normal_client_config(fixture.right.local_pin().unwrap().peer_id())
        .unwrap();
    let server = fixture.right.server_config().unwrap();
    revoke(&fixture.left, &fixture.right);
    let (left, right) = duplex(4096);
    let (left, right) = tokio::join!(config.connect(left), server.accept(right));
    assert_eq!(
        NormalSession::open(left.unwrap(), fixture.left.clone())
            .await
            .err(),
        Some(SessionError::Untrusted)
    );
    drop(right);
}

#[tokio::test(start_paused = true)]
async fn foreign_authority_with_same_remote_link_fails_reciprocal_hello_identity_check() {
    let fixture = Fixture::new();
    let other = PeerAuthority::session("other".into(), SystemTime::now()).unwrap();
    link(&other, &fixture.right);
    let (left, right) = fixture.connections(4096).await;
    let (left, right) = tokio::join!(
        NormalSession::open(left, other),
        NormalSession::open(right, fixture.right.clone())
    );
    assert_eq!(right.err(), Some(SessionError::Protocol));
    assert_eq!(left.err(), Some(SessionError::Protocol));
}

#[tokio::test(start_paused = true)]
async fn revocation_between_open_and_run_invalidates_projection_and_run() {
    let fixture = Fixture::new();
    let (left, right) = fixture.sessions(4096).await;
    revoke(&fixture.left, &fixture.right);
    assert_eq!(left.authenticated(), Err(SessionError::Untrusted));
    assert_eq!(
        left.run(pending()).await,
        SessionOutcome::Closed(SessionError::Untrusted)
    );
    drop(right);
}

#[tokio::test(start_paused = true)]
async fn normal_tls_rejects_unknown_client_wrong_server_and_revoked_client() {
    for mode in ["unknown_client", "wrong_server", "revoked_client"] {
        let fixture = Fixture::new();
        let stranger = Identity::generate(SystemTime::now()).unwrap();
        let config = match mode {
            "unknown_client" => crate::service::NormalClientConfig::new(
                &stranger,
                fixture.right.local_pin().unwrap(),
            )
            .unwrap(),
            "wrong_server" => {
                crate::service::NormalClientConfig::new(&stranger, stranger.pin().clone()).unwrap()
            }
            "revoked_client" => fixture
                .left
                .normal_client_config(fixture.right.local_pin().unwrap().peer_id())
                .unwrap(),
            _ => unreachable!(),
        };
        if mode == "revoked_client" {
            revoke(&fixture.right, &fixture.left);
        }
        let server = fixture.right.server_config().unwrap();
        let (left, right) = duplex(4096);
        let (left, right) = tokio::join!(config.connect(left), server.accept(right));
        if mode == "wrong_server" {
            assert!(
                matches!(left, Err(crate::service::PeerError::UntrustedPeer)),
                "{mode}"
            );
            continue;
        }
        assert!(
            matches!(right, Err(crate::service::PeerError::UntrustedPeer)),
            "{mode}"
        );
    }
}
