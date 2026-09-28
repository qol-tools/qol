use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::*;

const PEER: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const ACTIVATION: &str = "AAAAAAAAAAAAAAAAAAAAAA";

fn expected() -> Value {
    json!({"authority_id": PEER, "activation_id": ACTIVATION, "revision": "0"})
}

fn cursor() -> Value {
    json!({"authority_id": PEER, "activation_id": ACTIVATION, "revision": "0", "offset": 0})
}

fn operation() -> Value {
    json!({"identity": {"scope": "stable", "value": "fixture"}, "kind": "query", "name": "read"})
}

fn corruptions(value: &Value) -> Vec<String> {
    match value {
        Value::Object(fields) => {
            let wire = value.to_string();
            let prefix = &wire[..wire.len() - 1];
            let mut cases = vec![format!("{prefix},\"unexpected\":null}}")];
            for (key, child) in fields {
                let encoded_key = serde_json::to_string(key).unwrap();
                let escaped_key = format!("\"\\u{:04x}{}\"", key.as_bytes()[0], &key[1..]);
                cases.push(format!("{prefix},{encoded_key}:{child}}}"));
                cases.push(format!("{prefix},{escaped_key}:{child}}}"));
                for invalid in corruptions(child) {
                    let entries = fields
                        .iter()
                        .map(|(candidate, value)| {
                            let child = if candidate == key {
                                invalid.clone()
                            } else {
                                value.to_string()
                            };
                            format!("{}:{child}", serde_json::to_string(candidate).unwrap())
                        })
                        .collect::<Vec<_>>()
                        .join(",");
                    cases.push(format!("{{{entries}}}"));
                }
            }
            cases
        }
        Value::Array(items) => items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| {
                corruptions(item).into_iter().map(move |invalid| {
                    let items = items
                        .iter()
                        .enumerate()
                        .map(|(candidate, value)| {
                            if candidate == index {
                                invalid.clone()
                            } else {
                                value.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("[{items}]")
                })
            })
            .collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Vec::new(),
    }
}

fn strict<T: DeserializeOwned + Serialize>(fixture: Value) {
    let wire = fixture.to_string();
    let parsed: T = serde_json::from_str(&wire).unwrap_or_else(|error| panic!("{wire}: {error}"));
    assert_eq!(serde_json::to_value(parsed).unwrap(), fixture);
    for invalid in corruptions(&fixture) {
        assert!(
            serde_json::from_str::<T>(&invalid).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn every_request_rejects_unknown_and_duplicate_fields_recursively() {
    for fixture in [
        json!({"operation": "status"}),
        json!({"operation": "enable"}),
        json!({"operation": "network"}),
        json!({"operation": "sessions", "cursor": session_cursor()}),
        json!({"operation": "peers", "cursor": cursor()}),
        json!({"operation": "grants", "cursor": cursor(), "peer_id": PEER}),
        json!({"operation": "tombstones", "cursor": cursor()}),
        json!({"operation": "start_session", "name": "local"}),
        json!({"operation": "create_persistent", "name": "local"}),
        json!({"operation": "open_persistent"}),
        json!({"operation": "stop", "expected": expected()}),
        json!({"operation": "rename", "expected": expected(), "name": "local"}),
        json!({"operation": "set_grants", "expected": expected(), "peer_id": PEER, "grants": [operation()]}),
        json!({"operation": "revoke", "expected": expected(), "peer_id": PEER}),
    ] {
        strict::<Request>(fixture);
    }
}

#[test]
fn every_lifecycle_and_response_rejects_unknown_and_duplicate_fields_recursively() {
    for state in ["inactive", "standby", "active", "stopping", "shutdown"] {
        let lifecycle = json!({"state": state});
        strict::<Lifecycle>(lifecycle.clone());
        strict::<Response>(
            json!({"result": "status", "status": {"lifecycle": lifecycle, "authority": null}}),
        );
    }
    let authority = json!({
        "peer_id": PEER, "activation_id": ACTIVATION, "name": "local",
        "lifetime": "persistent", "revision": "0", "status": "ready",
        "peer_count": 1, "grant_count": 1, "tombstone_count": 1
    });
    for (result, item) in [
        (
            "peers",
            json!({"peer_id": PEER, "name": "remote", "grant_count": 1}),
        ),
        ("grants", operation()),
        ("tombstones", json!(PEER)),
    ] {
        let mut response = json!({"result": result, "page": {
            "cursor": cursor(), "total": 2, "items": [item], "next": cursor()
        }});
        if result == "grants" {
            response["peer_id"] = json!(PEER);
        }
        strict::<Response>(response);
    }
    strict::<Response>(json!({"result": "status", "status": {
        "lifecycle": {"state": "active"}, "authority": authority
    }}));
    strict::<Response>(
        json!({"result": "changed", "authority_id": PEER, "activation_id": ACTIVATION, "revision": "1"}),
    );
}

#[test]
fn every_error_is_strict_when_direct_or_nested_in_lifecycles_and_replies() {
    let authority_codes = [
        "faulted",
        "revision_exhausted",
        "capacity",
        "invalid_name",
        "invalid_grant",
        "duplicate_grant",
        "duplicate_peer",
        "unknown_peer",
        "revoked",
        "local_peer",
        "already_exists",
        "missing_store",
        "writer_busy",
        "unsafe_store",
        "invalid_snapshot",
        "unsupported_version",
        "identity",
        "unsupported_platform",
        "storage",
        "transport",
    ];
    let mut errors = [
        "inactive",
        "standby",
        "already_active",
        "stopping",
        "shutdown",
        "root_unavailable",
        "host_unavailable",
        "stale_authority",
        "activation_unavailable",
        "stale_cursor",
        "invalid_cursor",
        "grant_unavailable",
        "reply_too_large",
    ]
    .map(|code| json!({"code": code}))
    .to_vec();
    for error in authority_codes
        .into_iter()
        .map(|code| json!({"code": code}))
        .chain([json!({"code": "stale_revision", "expected": "0", "current": "1"})])
    {
        strict::<AuthorityError>(error.clone());
        errors.push(json!({"code": "authority", "error": error}));
    }
    for error in errors {
        strict::<Error>(error.clone());
        strict::<Response>(json!({"result": "error", "error": error}));
        let lifecycle = json!({"state": "unavailable", "error": error});
        strict::<Lifecycle>(lifecycle.clone());
        strict::<Response>(
            json!({"result": "status", "status": {"lifecycle": lifecycle, "authority": null}}),
        );
    }
}

#[test]
fn activation_and_expected_revision_have_only_canonical_encodings() {
    let id = ActivationId::from_bytes([0; 16]);
    assert_eq!(id.to_string(), ACTIVATION);
    assert_eq!(ACTIVATION.parse::<ActivationId>().unwrap(), id);
    for invalid in [
        "",
        "AAAAAAAAAAAAAAAAAAAAAA==",
        "AAAAAAAAAAAAAAAAAAAAAB",
        "AAAAAAAAAAAAAAAAAAAAA",
        "AAAAAAAAAAAAAAAAAAAAAAA",
        "++++++++++++++++++++++",
    ] {
        assert!(invalid.parse::<ActivationId>().is_err(), "{invalid}");
    }
    for revision in [
        json!(0),
        json!("01"),
        json!("+1"),
        json!("-1"),
        json!("18446744073709551616"),
    ] {
        let mut stamp = expected();
        stamp["revision"] = revision.clone();
        assert!(serde_json::from_value::<ExpectedAuthority>(stamp.clone()).is_err());
        assert!(
            serde_json::from_value::<Request>(json!({"operation": "stop", "expected": stamp}))
                .is_err()
        );
        assert!(serde_json::from_value::<AuthorityError>(
            json!({"code": "stale_revision", "expected": revision, "current": "1"})
        )
        .is_err());
    }
    for wire in [
        r#"{"state":"unknown"}"#,
        r#"{"state":"unavailable","error":{"code":"unknown"}}"#,
    ] {
        assert!(serde_json::from_str::<Lifecycle>(wire).is_err());
    }
    for wire in [
        r#"{"result":"unknown"}"#,
        r#"{"result":"error","error":{"code":"unknown"}}"#,
    ] {
        assert!(serde_json::from_str::<Response>(wire).is_err());
    }
    assert!(serde_json::from_str::<Request>(r#"{"operation":"unknown"}"#).is_err());
    assert!(serde_json::from_str::<AuthorityError>(r#"{"code":"unknown"}"#).is_err());
}

#[test]
fn activation_stamps_are_required_by_mutations_cursors_summaries_and_changed_replies() {
    for fixture in [
        json!({"operation": "stop", "expected": "0"}),
        json!({"operation": "rename", "expected": "0", "name": "local"}),
        json!({"operation": "set_grants", "expected": "0", "peer_id": PEER, "grants": []}),
        json!({"operation": "revoke", "expected": "0", "peer_id": PEER}),
    ] {
        assert!(serde_json::from_value::<Request>(fixture).is_err());
    }
    for missing in ["authority_id", "activation_id", "revision"] {
        let mut stamp = expected();
        stamp.as_object_mut().unwrap().remove(missing);
        assert!(serde_json::from_value::<ExpectedAuthority>(stamp).is_err());
    }
    assert!(serde_json::from_value::<PageCursor>(
        json!({"authority_id": PEER, "revision": "0", "offset": 0})
    )
    .is_err());
    assert!(serde_json::from_value::<Response>(
        json!({"result": "changed", "authority_id": PEER, "revision": "0"})
    )
    .is_err());
    assert!(serde_json::from_value::<AuthoritySummary>(json!({
        "peer_id": PEER, "name": "local", "lifetime": "session", "revision": "0",
        "status": "ready", "peer_count": 0, "grant_count": 0, "tombstone_count": 0
    }))
    .is_err());
}

fn session_cursor() -> Value {
    json!({"authority_id": PEER, "activation_id": ACTIVATION, "revision": "0", "network_revision": "0", "offset": 0})
}

#[test]
fn network_projection_schema_rejects_recursive_extras_and_duplicates() {
    use crate::network::{DiscoveryStatus, ListenerStatus};
    for state in ["starting", "ready", "closed"] {
        strict::<DiscoveryStatus>(json!({"state": state}));
    }
    for state in ["starting", "closed"] {
        strict::<ListenerStatus>(json!({"state": state}));
    }
    strict::<ListenerStatus>(json!({"state": "listening", "port": 1234}));
    for error in [
        "listener",
        "discovery",
        "authority",
        "cleanup",
        "task",
        "revision_exhausted",
        "runtime_unavailable",
    ] {
        strict::<DiscoveryStatus>(json!({"state": "failed", "error": error}));
        strict::<ListenerStatus>(json!({"state": "failed", "error": error}));
    }
    strict::<Response>(json!({"result": "network", "network": {
        "authority": expected(), "network_revision": "0", "status": {
            "family": "ipv4_only", "listener": {"state": "listening", "port": 1234},
            "enrollment_listener": {"state": "listening", "port": 5678},
            "discovery": {"state": "starting"}, "stopping": false, "failure": null
        }
    }}));
    strict::<Response>(json!({"result": "sessions", "page": {
        "cursor": session_cursor(), "total": 1, "items": [{"peer_id": PEER, "name": "remote", "generation": {
            "local": ACTIVATION, "remote": ACTIVATION
        }}], "next": session_cursor()
    }}));
}

#[test]
fn enrollment_requests_responses_and_attempt_states_are_strict_and_redacted() {
    let transaction = ACTIVATION;
    let key = json!({"invitation": ACTIVATION, "transaction": transaction, "peer": PEER});
    for request in [
        json!({"action":"create_invitation","expected":expected(),"addresses":["192.0.2.1"]}),
        json!({"action":"cancel_invitation","expected":expected(),"invitation":ACTIVATION}),
        json!({"action":"pending"}),
        json!({"action":"approve","expected":expected(),"key":key}),
        json!({"action":"reject","expected":expected(),"key":key}),
        json!({"action":"prepare","expected":expected(),"document":"qol-link:synthetic-canary"}),
        json!({"action":"redeem","expected":expected(),"transaction":transaction,"document":"qol-link:synthetic-canary"}),
        json!({"action":"recover","expected":expected(),"transaction":transaction,"endpoints":["192.0.2.1:1234"]}),
        json!({"action":"abandon","expected":expected(),"transaction":transaction}),
        json!({"action":"resume","expected":expected(),"transaction":transaction}),
        json!({"action":"outbound","cursor":cursor()}),
        json!({"action":"attempt","expected":expected(),"transaction":transaction}),
    ] {
        let fixture = json!({"operation":"enrollment","request":request});
        strict::<Request>(fixture.clone());
        let parsed: Request = serde_json::from_value(fixture).unwrap();
        assert!(!format!("{parsed:?}").contains("synthetic-canary"));
    }
    strict::<Response>(
        json!({"result":"invitation","authority":expected(),"invitation":ACTIVATION,"document":"qol-link:synthetic-canary"}),
    );
    strict::<Response>(
        json!({"result":"pending_enrollments","authority":expected(),"items":[{
            "key":key,"name":"remote","local_lifetime":"session","remote_lifetime":"persistent"
        }]}),
    );
    strict::<Response>(
        json!({"result":"join_prepared","authority":expected(),"transaction":transaction}),
    );
    for state in [json!({"kind":"pending"}), json!({"kind":"abandoned"})] {
        strict::<Response>(json!({"result":"outbound_enrollments","page":{
            "cursor":cursor(),"total":1,"items":[{"key":key,"state":state}],"next":null
        }}));
    }
    let receipt = json!({"invitation":ACTIVATION,"transaction":transaction,"inviter":PEER,
        "joiner":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE", "inviter_name":"left", "joiner_name":"right",
        "inviter_lifetime":"session", "joiner_lifetime":"persistent"});
    for state in [
        json!({"state":"unavailable"}),
        json!({"state":"queued"}),
        json!({"state":"running"}),
        json!({"state":"completed","receipt":receipt}),
        json!({"state":"rejected","reason":"cancelled"}),
        json!({"state":"unknown","reason":"transport"}),
    ] {
        strict::<Response>(
            json!({"result":"enrollment_attempt","authority":expected(),"transaction":transaction,"state":state}),
        );
    }
    for error in [
        json!("invalid_invitation"),
        json!("invalid_endpoint"),
        json!("unsupported_address_family"),
        json!("unavailable"),
        json!("capacity"),
        json!("already_running"),
        json!("unknown_transaction"),
        json!("abandoned"),
        json!("transport"),
        json!("protocol"),
        json!({"rejected":"revoked"}),
    ] {
        strict::<Error>(json!({"code":"enrollment","error":error}));
    }
}

#[test]
fn invitation_document_limits_and_debug_never_expose_secret_input() {
    use crate::enrollment::{ExportedInvitation, MAX_INVITATION_BYTES};
    use zeroize::Zeroizing;
    for length in [
        MAX_INVITATION_BYTES - 1,
        MAX_INVITATION_BYTES,
        MAX_INVITATION_BYTES + 1,
    ] {
        let value = format!("qol-link:{}", "x".repeat(length - "qol-link:".len()));
        let result = ExportedInvitation::from_owned(Zeroizing::new(value));
        assert_eq!(result.is_ok(), length <= MAX_INVITATION_BYTES, "{length}");
    }
    let document =
        ExportedInvitation::from_owned(Zeroizing::new("qol-link:synthetic-canary".into())).unwrap();
    assert_eq!(format!("{document:?}"), "ExportedInvitation([REDACTED])");
    let request = Request::Enrollment {
        request: EnrollmentRequest::Prepare {
            expected: serde_json::from_value(expected()).unwrap(),
            document,
        },
    };
    assert!(request.is_mutation());
    assert!(!format!("{request:?}").contains("synthetic-canary"));
    assert!(!Request::Enrollment {
        request: EnrollmentRequest::Pending {}
    }
    .is_mutation());
}

#[test]
fn nearby_requests_and_replies_are_strict() {
    for request in [
        json!({"action": "list"}),
        json!({"action": "link", "expected": expected(), "peer_id": PEER, "grants": [operation()]}),
        json!({"action": "confirm", "expected": expected(), "peer_id": PEER, "grants": [operation()]}),
        json!({"action": "decline", "expected": expected(), "peer_id": PEER}),
    ] {
        strict::<Request>(json!({"operation": "nearby", "request": request}));
    }
    for link in [
        json!(null),
        json!({"code": null, "state": {"state": "connecting"}}),
        json!({"code": 42, "state": {"state": "confirm"}}),
        json!({"code": 999_999, "state": {"state": "waiting_for_peer"}}),
        json!({"code": 1, "state": {"state": "failed", "error": "transport"}}),
    ] {
        strict::<Response>(
            json!({"result": "nearby", "authority": expected(), "computers": [
                {"peer_id": PEER, "name": "Desk", "link": link}
            ]}),
        );
    }
    for invalid in [
        json!({"peer_id": PEER, "name": "Desk", "link": {"code": 1_000_000, "state": {"state": "confirm"}}}),
        json!({"peer_id": PEER, "name": "", "link": null}),
        json!({"peer_id": PEER, "name": " Desk", "link": null}),
    ] {
        assert!(serde_json::from_value::<NearbyComputer>(invalid).is_err());
    }
    assert_eq!(LinkCode::new(7_042).unwrap().to_string(), "007 042");
}
