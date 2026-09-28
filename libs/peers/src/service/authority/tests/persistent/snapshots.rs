use std::fs;

use crate::service::TrustPolicy;

use serde_json::{json, Value};

use super::{
    grant, now, persistent, pin, revision, wire, write_wire, AuthorityError, PeerAuthority,
};
use crate::service::authority::state::MAX_SNAPSHOT_BYTES;

#[test]
fn malformed_snapshots_are_rejected_without_repair_or_identity_replacement() {
    let cases = [
        "duplicate_peer",
        "duplicate_grant",
        "missing_pin",
        "mismatched_peer_id",
        "mismatched_local_id",
        "invalid_pin",
        "invalid_key",
        "wrong_key",
        "version",
        "unknown_field",
        "local_peer",
        "local_grant",
        "wildcard",
        "empty_name",
        "too_many_peers",
        "too_many_grants",
        "too_many_tombstones",
        "oversized_key",
        "unknown_operation_field",
        "unknown_operation_kind",
    ];
    for case in cases {
        let (_temporary, root, authority) = persistent();
        authority
            .insert_link(revision(&authority), pin(1), "remote".into())
            .unwrap();
        authority
            .set_grants(revision(&authority), pin(1).peer_id(), vec![grant("run")])
            .unwrap();
        drop(authority);
        let mut value = wire(&root);
        corrupt(case, &mut value);
        write_wire(&root, &value);
        let bytes = fs::read(root.join("state.json")).unwrap();
        let result = PeerAuthority::open_persistent(&root, now());
        assert!(result.is_err(), "accepted {case}");
        let error = result.err().unwrap();
        if case == "version" {
            assert_eq!(error, AuthorityError::UnsupportedVersion);
        }
        if case == "wrong_key" || case == "invalid_key" {
            assert_eq!(error, AuthorityError::Identity);
        }
        assert!(
            !format!("{error:?} {error}").contains("private-key-sentinel"),
            "{case}"
        );
        assert_eq!(
            fs::read(root.join("state.json")).unwrap(),
            bytes,
            "repaired {case}"
        );
    }
}

fn corrupt(case: &str, value: &mut Value) {
    let peer = value["peers"][0].clone();
    let stored_pin = peer["pin"].clone();
    match case {
        "duplicate_peer" => value["peers"].as_array_mut().unwrap().push(peer),
        "duplicate_grant" => value["peers"][0]["grants"]
            .as_array_mut()
            .unwrap()
            .push(peer["grants"][0].clone()),
        "missing_pin" => {
            value["peers"][0].as_object_mut().unwrap().remove("pin");
        }
        "mismatched_peer_id" => value["peers"][0]["pin"]["peer_id"] = json!(pin(2).peer_id()),
        "mismatched_local_id" => value["identity"]["peer_id"] = json!(pin(2).peer_id()),
        "invalid_pin" => value["peers"][0]["pin"]["spki"] = json!([0]),
        "invalid_key" => value["identity"]["key"] = json!([0]),
        "wrong_key" => {
            let identity = crate::service::Identity::generate(now()).unwrap();
            value["identity"]["key"] = json!(identity.export_secret().expose_pkcs8());
        }
        "version" => value["version"] = json!(1),
        "unknown_field" => value["private-key-sentinel"] = json!(true),
        "local_peer" => {
            let bytes: Vec<u8> =
                serde_json::from_value(value["identity"]["certificate"].clone()).unwrap();
            let local = crate::service::validate_certificate(
                &bytes,
                now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .unwrap();
            value["peers"][0]["pin"] =
                json!({"peer_id": local.peer_id(), "spki": local.spki_der()});
        }
        "local_grant" => value["peers"][0]["grants"][0]["identity"]["scope"] = json!("local"),
        "wildcard" => value["peers"][0]["grants"][0]["name"] = json!("*"),
        "empty_name" => value["name"] = json!(""),
        "too_many_peers" => value["peers"] = json!(vec![peer; 257]),
        "too_many_grants" => {
            value["peers"][0]["grants"] = json!(vec![peer["grants"][0].clone(); 129])
        }
        "too_many_tombstones" => value["tombstones"] = json!(vec![stored_pin; 4097]),
        "oversized_key" => value["identity"]["key"] = json!(vec![0_u8; 4097]),
        "unknown_operation_field" => value["peers"][0]["grants"][0]["other"] = json!(true),
        "unknown_operation_kind" => value["peers"][0]["grants"][0]["kind"] = json!("admin"),
        other => panic!("unknown fixture {other}"),
    }
}

#[test]
fn a_device_blocked_by_an_older_unlink_can_link_again() {
    let (_temporary, root, authority) = persistent();
    drop(authority);
    let mut value = wire(&root);
    let blocked = pin(1);
    value["tombstones"] = json!([{"peer_id": blocked.peer_id(), "spki": blocked.spki_der()}]);
    write_wire(&root, &value);
    let reopened = PeerAuthority::open_persistent(&root, now()).unwrap();
    assert!(reopened.projection().unwrap().tombstones.is_empty());
    assert_eq!(wire(&root)["tombstones"], json!([]));
    reopened
        .insert_link(revision(&reopened), blocked.clone(), "again".into())
        .unwrap();
    assert!(reopened.is_trusted(&blocked));
}

#[test]
fn missing_corrupt_duplicate_field_and_oversized_snapshots_never_bootstrap() {
    for case in [
        "missing",
        "missing_lock",
        "corrupt",
        "duplicate_field",
        "oversized",
    ] {
        let (_temporary, root, authority) = persistent();
        drop(authority);
        let path = root.join("state.json");
        match case {
            "missing" => fs::remove_file(&path).unwrap(),
            "missing_lock" => fs::remove_file(root.join("writer.lock")).unwrap(),
            "corrupt" => fs::write(&path, b"private-key-sentinel").unwrap(),
            "duplicate_field" => {
                let bytes = fs::read_to_string(&path).unwrap();
                fs::write(&path, format!("{{\"version\":3,{}", &bytes[1..])).unwrap();
            }
            "oversized" => fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(MAX_SNAPSHOT_BYTES as u64 + 1)
                .unwrap(),
            other => panic!("unknown fixture {other}"),
        }
        let result = PeerAuthority::open_persistent(&root, now());
        let error = result.err().unwrap();
        let expected = match case {
            "missing" | "missing_lock" => AuthorityError::MissingStore,
            "corrupt" | "duplicate_field" => AuthorityError::InvalidSnapshot,
            "oversized" => AuthorityError::Capacity,
            other => panic!("unknown fixture {other}"),
        };
        assert_eq!(error, expected, "{case}");
        if case == "missing" {
            assert!(!path.exists());
        }
        if case == "missing_lock" {
            assert!(!root.join("writer.lock").exists());
        }
    }
}
