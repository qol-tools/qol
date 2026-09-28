mod failures;
mod files;
mod snapshots;

use std::{fs, path::PathBuf, time::Duration};

use tempfile::TempDir;

use super::{grant, now, pin, revision, AuthorityError, PeerAuthority};
use crate::service::TrustPolicy;
use crate::{AuthorityLifetime, StoreRevision};

fn persistent() -> (TempDir, PathBuf, PeerAuthority) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("authority");
    let authority = PeerAuthority::create_persistent(&root, "local".into(), now()).unwrap();
    (temporary, root, authority)
}

fn wire(root: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(root.join("state.json")).unwrap()).unwrap()
}

fn write_wire(root: &std::path::Path, value: &serde_json::Value) {
    fs::write(root.join("state.json"), serde_json::to_vec(value).unwrap()).unwrap();
}

#[test]
fn restart_preserves_identity_grants_names_revision_and_irreversible_revocation() {
    let (_temporary, root, authority) = persistent();
    let local_pin = authority.local_pin().unwrap();
    let remote = pin(1);
    authority
        .insert_link(revision(&authority), remote.clone(), "remote".into())
        .unwrap();
    authority
        .set_grants(revision(&authority), remote.peer_id(), vec![grant("run")])
        .unwrap();
    authority
        .rename(revision(&authority), "renamed".into())
        .unwrap();
    let before = authority.projection().unwrap();
    drop(authority);
    let reopened = PeerAuthority::open_persistent(&root, now()).unwrap();
    assert_eq!(reopened.projection().unwrap(), before);
    assert_eq!(reopened.local_pin().unwrap(), local_pin);
    assert_eq!(before.lifetime, AuthorityLifetime::Persistent);
    assert!(reopened.is_trusted(&remote));
    assert!(reopened.has_grant(remote.peer_id(), &grant("run")));
    reopened
        .set_grants(
            revision(&reopened),
            remote.peer_id(),
            vec![grant("replacement")],
        )
        .unwrap();
    drop(reopened);
    let reopened = PeerAuthority::open_persistent(&root, now()).unwrap();
    assert!(!reopened.has_grant(remote.peer_id(), &grant("run")));
    assert!(reopened.has_grant(remote.peer_id(), &grant("replacement")));
    reopened
        .revoke(revision(&reopened), remote.peer_id())
        .unwrap();
    drop(reopened);
    let reopened = PeerAuthority::open_persistent(&root, now()).unwrap();
    assert_eq!(reopened.local_pin().unwrap(), local_pin);
    assert!(!reopened.is_trusted(&remote));
    assert!(!reopened.has_grant(remote.peer_id(), &grant("replacement")));
    assert_eq!(
        reopened.insert_link(revision(&reopened), remote.clone(), "again".into()),
        Err(AuthorityError::Revoked)
    );
    assert_eq!(
        reopened.projection().unwrap().tombstones,
        vec![remote.peer_id()]
    );
}

#[test]
fn concurrent_handle_mutations_accept_exactly_one_expected_revision() {
    use std::sync::{Arc, Barrier};
    let (_temporary, root, authority) = persistent();
    let barrier = Arc::new(Barrier::new(3));
    let expected = revision(&authority);
    let workers: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|name| {
            let authority = authority.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                authority.rename(expected, name.into())
            })
        })
        .collect();
    barrier.wait();
    let outcomes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| matches!(result, Err(AuthorityError::StaleRevision { .. })))
            .count(),
        1
    );
    let committed = authority.projection().unwrap();
    assert_eq!(committed.revision.value(), expected.value() + 1);
    drop(authority);
    assert_eq!(
        PeerAuthority::open_persistent(&root, now())
            .unwrap()
            .projection()
            .unwrap(),
        committed
    );
}

#[test]
fn expired_local_certificate_renews_same_key_and_commits_before_returning() {
    let (_temporary, root, authority) = persistent();
    let before = wire(&root);
    let original_pin = authority.local_pin().unwrap();
    let remote = pin(1);
    authority
        .insert_link(revision(&authority), remote.clone(), "remote".into())
        .unwrap();
    authority
        .set_grants(revision(&authority), remote.peer_id(), vec![grant("run")])
        .unwrap();
    let prior_revision = revision(&authority);
    drop(authority);
    let later = now() + Duration::from_secs(367 * 24 * 60 * 60);
    let reopened = PeerAuthority::open_persistent(&root, later).unwrap();
    let committed = wire(&root);
    assert_eq!(reopened.local_pin().unwrap(), original_pin);
    assert_eq!(revision(&reopened).value(), prior_revision.value() + 1);
    assert_eq!(committed["revision"], revision(&reopened).to_string());
    assert_ne!(
        committed["identity"]["certificate"],
        before["identity"]["certificate"]
    );
    assert_eq!(committed["identity"]["key"], before["identity"]["key"]);
    assert!(reopened.has_grant(remote.peer_id(), &grant("run")));
    let projection = reopened.projection().unwrap();
    drop(reopened);
    let again = PeerAuthority::open_persistent(&root, later).unwrap();
    assert_eq!(again.projection().unwrap(), projection);
    assert_eq!(wire(&root), committed);
}

#[test]
fn revision_overflow_and_tombstone_capacity_leave_durable_state_unchanged() {
    for exhausted in ["revision", "tombstones"] {
        let (_temporary, root, authority) = persistent();
        let remote = pin(1);
        authority
            .insert_link(revision(&authority), remote.clone(), "remote".into())
            .unwrap();
        authority
            .set_grants(revision(&authority), remote.peer_id(), vec![grant("run")])
            .unwrap();
        drop(authority);
        let mut value = wire(&root);
        if exhausted == "revision" {
            value["revision"] = u64::MAX.to_string().into();
        }
        if exhausted == "tombstones" {
            value["tombstones"] = serde_json::Value::Array(
                (2..4098)
                    .map(|index| {
                        let pin = pin(index);
                        serde_json::json!({"peer_id": pin.peer_id(), "spki": pin.spki_der()})
                    })
                    .collect(),
            );
        }
        write_wire(&root, &value);
        let bytes = fs::read(root.join("state.json")).unwrap();
        let authority = PeerAuthority::open_persistent(&root, now()).unwrap();
        let expected = if exhausted == "revision" {
            AuthorityError::RevisionExhausted
        } else {
            AuthorityError::Capacity
        };
        assert_eq!(
            authority.revoke(revision(&authority), remote.peer_id()),
            Err(expected),
            "{exhausted}"
        );
        assert!(authority.is_trusted(&remote), "{exhausted}");
        assert!(
            authority.has_grant(remote.peer_id(), &grant("run")),
            "{exhausted}"
        );
        let before = authority.projection().unwrap();
        if exhausted == "revision" {
            assert_eq!(
                authority.rename(StoreRevision::new(u64::MAX), "overflow".into()),
                Err(expected)
            );
        }
        assert_eq!(
            fs::read(root.join("state.json")).unwrap(),
            bytes,
            "{exhausted}"
        );
        drop(authority);
        assert_eq!(
            PeerAuthority::open_persistent(&root, now())
                .unwrap()
                .projection()
                .unwrap(),
            before,
            "{exhausted}"
        );
    }
}
