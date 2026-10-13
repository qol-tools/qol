#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod persistent;
mod tls;

use std::time::{Duration, SystemTime};

use p256::elliptic_curve::sec1::ToEncodedPoint;
use qol_conventions::{
    operations::{OperationIdentity, OperationKey, OperationKind},
    plugin_id::{PluginId, PluginUid},
};

use super::{
    state::{MAX_GRANTS, MAX_PEERS},
    AuthorityError, PeerAuthority,
};
use crate::service::{Identity, PeerPin, TrustPolicy};
use crate::{AuthorityLifetime, AuthorityStatus, StoreRevision};

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

fn grant(name: &str) -> OperationKey {
    OperationKey::new(
        PluginUid::new("immutable-fixture-uid"),
        OperationKind::Action,
        name,
    )
}

fn pin(index: u32) -> PeerPin {
    let mut scalar = [0_u8; 32];
    scalar[28..].copy_from_slice(&index.to_be_bytes());
    let key = p256::SecretKey::from_slice(&scalar).unwrap();
    let point = key.public_key().to_encoded_point(false);
    let mut spki = vec![
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    spki.extend_from_slice(point.as_bytes());
    PeerPin::from_spki_der(&spki).unwrap()
}

fn revision(authority: &PeerAuthority) -> StoreRevision {
    authority.projection().unwrap().revision
}

#[test]
fn session_lifecycle_exact_grants_and_revocation_share_one_state() {
    let authority = PeerAuthority::session("local".into(), now()).unwrap();
    let observer = authority.clone();
    let remote = Identity::generate(now()).unwrap();
    let id = remote.pin().peer_id();
    let first = authority
        .insert_link(
            StoreRevision::INITIAL,
            remote.pin().clone(),
            "remote".into(),
        )
        .unwrap();
    assert!(observer.is_trusted(remote.pin()));
    assert!(!observer.has_grant(id, &grant("run")));
    let second = authority.set_grants(first, id, vec![grant("run")]).unwrap();
    assert!(observer.has_grant(id, &grant("run")));
    for other in [
        grant("future"),
        OperationKey::new(PluginUid::new("another-uid"), OperationKind::Action, "run"),
        OperationKey::new(
            PluginUid::new("immutable-fixture-uid"),
            OperationKind::Query,
            "run",
        ),
    ] {
        assert!(!observer.has_grant(id, &other), "{other:?}");
    }
    let third = authority.revoke(second, id).unwrap();
    assert!(!observer.is_trusted(remote.pin()));
    assert!(!observer.has_grant(id, &grant("run")));
    assert_eq!(
        authority.set_grants(third, id, vec![grant("run")]),
        Err(AuthorityError::UnknownPeer)
    );
    let projection = authority.projection().unwrap();
    assert_eq!(projection.lifetime, AuthorityLifetime::Session);
    assert_eq!(projection.status, AuthorityStatus::Ready);
    assert!(projection.peers.is_empty());
    let wire = serde_json::to_value(&projection).unwrap();
    assert_eq!(wire["revision"], "3");
    assert!(wire.get("identity").is_none());
    assert!(wire.get("key").is_none());
    assert!(wire.get("certificate").is_none());
    let mut detached = projection;
    detached.name = "changed copy".into();
    assert_eq!(observer.projection().unwrap().name, "local");
}

#[test]
fn invalid_mutations_and_stale_revisions_never_change_state() {
    let authority = PeerAuthority::session("local".into(), now()).unwrap();
    let remote = pin(1);
    let local = authority.local_pin().unwrap();
    assert_eq!(
        authority.insert_link(revision(&authority), local, "self".into()),
        Err(AuthorityError::LocalPeer)
    );
    for name in ["", " ", " leading", "trailing ", "a\nb", "a\0b"] {
        assert_eq!(
            authority.insert_link(revision(&authority), remote.clone(), name.into()),
            Err(AuthorityError::InvalidName),
            "{name:?}"
        );
    }
    authority
        .insert_link(revision(&authority), remote.clone(), "remote".into())
        .unwrap();
    let before = authority.projection().unwrap();
    assert_eq!(
        authority.insert_link(before.revision, remote.clone(), "duplicate".into()),
        Err(AuthorityError::DuplicatePeer)
    );
    assert!(matches!(
        authority.rename(StoreRevision::INITIAL, "stale".into()),
        Err(AuthorityError::StaleRevision { .. })
    ));
    assert_eq!(
        authority.revoke(before.revision, pin(2).peer_id()),
        Err(AuthorityError::UnknownPeer)
    );
    assert_eq!(
        authority.set_grants(before.revision, pin(2).peer_id(), vec![]),
        Err(AuthorityError::UnknownPeer)
    );
    for name in ["", "\0", " x", "x "] {
        assert_eq!(
            authority.rename(before.revision, name.into()),
            Err(AuthorityError::InvalidName),
            "{name:?}"
        );
    }
    assert_eq!(authority.projection().unwrap(), before);
    assert_eq!(
        authority.rename(before.revision, "x".repeat(257)),
        Err(AuthorityError::InvalidName)
    );
    authority.rename(before.revision, "x".repeat(256)).unwrap();
    assert_eq!(authority.projection().unwrap().name.len(), 256);
}

#[test]
fn grants_reject_local_wildcard_invalid_duplicate_and_over_capacity_inputs() {
    let authority = PeerAuthority::session("local".into(), now()).unwrap();
    let remote = pin(1);
    authority
        .insert_link(revision(&authority), remote.clone(), "remote".into())
        .unwrap();
    let before = authority.projection().unwrap();
    let cases = [
        OperationKey {
            identity: OperationIdentity::Local(PluginId::new("label")),
            kind: OperationKind::Action,
            name: "run".into(),
        },
        grant("*"),
        grant("run.*"),
        grant("-run"),
        grant(""),
        grant(&"a".repeat(65)),
        OperationKey::new(PluginUid::new("*"), OperationKind::Action, "run"),
        OperationKey::new(PluginUid::new(" bad"), OperationKind::Action, "run"),
        OperationKey::new(
            PluginUid::new("x".repeat(257)),
            OperationKind::Action,
            "run",
        ),
        OperationKey::new(PluginUid::new("uid"), OperationKind::Query, "Upper"),
        OperationKey::new(PluginUid::new("uid"), OperationKind::Stream, "a-b"),
    ];
    for invalid in cases {
        assert_eq!(
            authority.set_grants(before.revision, remote.peer_id(), vec![invalid.clone()]),
            Err(AuthorityError::InvalidGrant),
            "{invalid:?}"
        );
        assert!(!authority.has_grant(remote.peer_id(), &invalid));
    }
    assert_eq!(
        authority.set_grants(
            before.revision,
            remote.peer_id(),
            vec![grant("run"), grant("run")]
        ),
        Err(AuthorityError::DuplicateGrant)
    );
    assert_eq!(
        authority.set_grants(
            before.revision,
            remote.peer_id(),
            (0..=MAX_GRANTS)
                .map(|i| grant(&format!("run_{i}")))
                .collect()
        ),
        Err(AuthorityError::Capacity)
    );
    assert_eq!(authority.projection().unwrap(), before);
    let maximum: Vec<_> = (0..MAX_GRANTS)
        .map(|i| grant(&format!("run_{i}")))
        .collect();
    authority
        .set_grants(before.revision, remote.peer_id(), maximum.clone())
        .unwrap();
    assert!(maximum
        .iter()
        .all(|key| authority.has_grant(remote.peer_id(), key)));
    authority
        .set_grants(revision(&authority), remote.peer_id(), vec![])
        .unwrap();
    assert!(maximum
        .iter()
        .all(|key| !authority.has_grant(remote.peer_id(), key)));
}

#[test]
fn peer_capacity_is_checked_without_partial_insertion() {
    let authority = PeerAuthority::session("local".into(), now()).unwrap();
    for index in 1..=MAX_PEERS {
        authority
            .insert_link(
                revision(&authority),
                pin(index as u32),
                format!("peer-{index}"),
            )
            .unwrap();
    }
    let before = authority.projection().unwrap();
    let overflow = pin(MAX_PEERS as u32 + 1);
    assert_eq!(
        authority.insert_link(before.revision, overflow.clone(), "overflow".into()),
        Err(AuthorityError::Capacity)
    );
    assert_eq!(authority.projection().unwrap(), before);
    assert!(!authority.is_trusted(&overflow));
}

#[test]
fn revision_wire_rejects_noncanonical_and_numeric_representations() {
    for wire in ["0", "1", "18446744073709551615"] {
        let revision: StoreRevision = wire.parse().unwrap();
        assert_eq!(
            serde_json::to_string(&revision).unwrap(),
            format!("\"{wire}\"")
        );
    }
    for wire in [
        "\"\"",
        "\"00\"",
        "\"01\"",
        "\"+1\"",
        "\"-1\"",
        "\" 1\"",
        "\"1 \"",
        "\"18446744073709551616\"",
        "1",
        "null",
    ] {
        assert!(
            serde_json::from_str::<StoreRevision>(wire).is_err(),
            "{wire}"
        );
    }
}
