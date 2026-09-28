use std::{
    fs,
    time::{Duration, SystemTime},
};

use super::{grant, now, persistent, pin, revision, wire, AuthorityError, PeerAuthority};
use crate::service::authority::storage::{CommitFault, Storage};
use crate::{
    service::{Identity, NormalClientConfig, PeerError, TrustPolicy},
    AuthorityStatus,
};

#[test]
fn failed_commit_denies_all_trust_and_grants_until_durable_reconciliation() {
    for (fault, committed) in [
        (CommitFault::BeforeReplace, false),
        (CommitFault::AfterReplace, true),
    ] {
        let (_temporary, root, authority) = persistent();
        for index in [1, 2] {
            authority
                .insert_link(revision(&authority), pin(index), format!("remote-{index}"))
                .unwrap();
            authority
                .set_grants(
                    revision(&authority),
                    pin(index).peer_id(),
                    vec![grant("run")],
                )
                .unwrap();
        }
        let clone = authority.clone();
        let before = authority.projection().unwrap();
        authority.inner.lock().unwrap().storage.fail_next(fault);
        assert_eq!(
            authority.revoke(before.revision, pin(1).peer_id()),
            Err(AuthorityError::Storage)
        );
        for index in [1, 2] {
            assert!(
                !clone.is_trusted(&pin(index)),
                "committed={committed}, peer={index}"
            );
            assert!(
                !clone.has_grant(pin(index).peer_id(), &grant("run")),
                "committed={committed}, peer={index}"
            );
        }
        let faulted = clone.projection().unwrap();
        assert_eq!(faulted.status, AuthorityStatus::Faulted);
        assert_eq!(faulted.revision, before.revision);
        assert_eq!(faulted.peers, before.peers);
        assert_eq!(
            clone.rename(before.revision, "blocked".into()),
            Err(AuthorityError::Faulted)
        );
        assert_eq!(
            clone.insert_link(before.revision, pin(3), "blocked".into()),
            Err(AuthorityError::Faulted)
        );
        drop(authority);
        assert!(matches!(
            PeerAuthority::open_persistent(&root, now()),
            Err(AuthorityError::WriterBusy)
        ));
        drop(clone);
        let reopened = PeerAuthority::open_persistent(&root, now()).unwrap();
        assert_eq!(reopened.is_trusted(&pin(1)), !committed);
        assert_eq!(
            reopened.has_grant(pin(1).peer_id(), &grant("run")),
            !committed
        );
        assert!(reopened.is_trusted(&pin(2)));
        assert!(reopened.has_grant(pin(2).peer_id(), &grant("run")));
        assert_eq!(
            revision(&reopened).value(),
            before.revision.value() + u64::from(committed)
        );
        if committed {
            assert_eq!(
                reopened.insert_link(revision(&reopened), pin(1), "revived".into()),
                Err(AuthorityError::Revoked)
            );
        }
    }
}

#[test]
fn renewal_failure_never_returns_an_uncommitted_authority() {
    for (fault, committed) in [
        (CommitFault::BeforeReplace, false),
        (CommitFault::AfterReplace, true),
    ] {
        let (_temporary, root, authority) = persistent();
        let original_pin = authority.local_pin().unwrap();
        drop(authority);
        let before = wire(&root);
        let mut storage = Storage::open(&root).unwrap();
        storage.fail_next(fault);
        let later = now() + Duration::from_secs(367 * 24 * 60 * 60);
        assert!(matches!(
            PeerAuthority::open_with_storage(storage, later),
            Err(AuthorityError::Storage)
        ));
        let after_failure = wire(&root);
        assert_eq!(
            after_failure["identity"]["certificate"] != before["identity"]["certificate"],
            committed
        );
        let reconciled = PeerAuthority::open_persistent(&root, later).unwrap();
        assert_eq!(reconciled.local_pin().unwrap(), original_pin);
        assert_eq!(revision(&reconciled).value(), 1);
        assert_eq!(wire(&root)["revision"], "1");
    }
}

#[test]
fn actual_storage_failure_faults_without_publishing_candidate() {
    let (_temporary, root, authority) = persistent();
    authority
        .insert_link(revision(&authority), pin(1), "remote".into())
        .unwrap();
    let before = authority.projection().unwrap();
    let old_bytes = fs::read(root.join("state.json")).unwrap();
    fs::rename(root.join("writer.lock"), root.join("moved-lock")).unwrap();
    assert!(authority
        .rename(before.revision, "candidate".into())
        .is_err());
    assert_eq!(
        authority.projection().unwrap().status,
        AuthorityStatus::Faulted
    );
    assert_eq!(authority.projection().unwrap().name, before.name);
    assert!(!authority.is_trusted(&pin(1)));
    assert_eq!(fs::read(root.join("state.json")).unwrap(), old_bytes);
}

#[tokio::test]
async fn existing_tls_configuration_denies_after_uncertain_storage_failure() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("authority");
    let now = SystemTime::now() - Duration::from_secs(60);
    let authority = PeerAuthority::create_persistent(&root, "server".into(), now).unwrap();
    let client_identity = Identity::generate(now).unwrap();
    authority
        .insert_link(
            revision(&authority),
            client_identity.pin().clone(),
            "client".into(),
        )
        .unwrap();
    let client = NormalClientConfig::new(&client_identity, authority.local_pin().unwrap()).unwrap();
    let server = authority.server_config().unwrap();
    authority
        .inner
        .lock()
        .unwrap()
        .storage
        .fail_next(CommitFault::AfterReplace);
    assert_eq!(
        authority.rename(revision(&authority), "failed".into()),
        Err(AuthorityError::Storage)
    );
    let (client_io, server_io) = tokio::io::duplex(4096);
    let (_, rejected) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(client.connect(client_io), server.accept(server_io))
    })
    .await
    .unwrap();
    assert!(matches!(rejected, Err(PeerError::UntrustedPeer)));
}
