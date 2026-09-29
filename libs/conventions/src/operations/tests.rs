use super::*;
use proptest::prelude::*;
use serde_json::json;
use std::collections::HashSet;

#[test]
fn operation_keys_use_the_canonical_wire_schema() {
    for (scope, identity) in [
        ("stable", OperationIdentity::Stable(PluginUid::new("same"))),
        ("local", OperationIdentity::Local(PluginId::new("same"))),
    ] {
        for (kind, tag) in [
            (OperationKind::Action, "action"),
            (OperationKind::Query, "query"),
            (OperationKind::Stream, "stream"),
        ] {
            let key = OperationKey {
                identity: identity.clone(),
                kind,
                name: "operation_name".into(),
            };
            let wire = json!({
                "identity": {"scope": scope, "value": "same"},
                "kind": tag,
                "name": "operation_name",
            });
            assert_eq!(serde_json::to_value(&key).unwrap(), wire, "{scope}/{tag}");
            assert_eq!(
                serde_json::from_value::<OperationKey>(wire).unwrap(),
                key,
                "{scope}/{tag}"
            );
        }
    }
}

#[test]
fn invocation_uses_snake_case_tags() {
    for (invocation, tag) in [(Invocation::Action, "action"), (Invocation::Query, "query")] {
        assert_eq!(
            serde_json::to_value(invocation).unwrap(),
            json!(tag),
            "{tag}"
        );
        assert_eq!(
            serde_json::from_value::<Invocation>(json!(tag)).unwrap(),
            invocation,
            "{tag}"
        );
    }
    for wire in [
        json!("stream"),
        json!("Action"),
        json!("unknown"),
        json!(null),
    ] {
        assert!(
            serde_json::from_value::<Invocation>(wire.clone()).is_err(),
            "{wire}"
        );
    }
}

#[test]
fn identity_rejects_unknown_missing_duplicate_and_mistyped_fields() {
    for wire in [
        r#"{"scope":"unknown","value":"uid"}"#,
        r#"{"scope":"Stable","value":"uid"}"#,
        r#"{"extra":true,"scope":"stable","value":"uid"}"#,
        r#"{"scope":"stable","extra":true,"value":"uid"}"#,
        r#"{"scope":"local","value":"id","extra":true}"#,
        r#"{"scope":"stable"}"#,
        r#"{"value":"uid"}"#,
        r#"{"scope":"stable","scope":"local","value":"uid"}"#,
        r#"{"scope":"stable","value":"uid","value":"other"}"#,
        r#"{"scope":"stable","value":null}"#,
        r#"{"scope":"local","value":7}"#,
        r#"{"stable":"uid"}"#,
    ] {
        assert!(
            serde_json::from_str::<OperationIdentity>(wire).is_err(),
            "{wire}"
        );
    }
}

#[test]
fn operation_key_rejects_unknown_missing_duplicate_and_mistyped_fields() {
    let valid = json!({
        "identity": {"scope": "stable", "value": "uid"},
        "kind": "action",
        "name": "operation_name",
    });
    for field in ["identity", "kind", "name"] {
        let mut wire = valid.clone();
        wire.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<OperationKey>(wire).is_err(),
            "missing {field}"
        );
    }
    for (field, value) in [
        ("extra", json!(true)),
        ("kind", json!("unknown")),
        ("kind", json!("Action")),
        ("kind", json!(null)),
        ("name", json!(17)),
        (
            "identity",
            json!({"scope": "local", "value": "id", "extra": 1}),
        ),
    ] {
        let mut wire = valid.clone();
        wire[field] = value;
        assert!(
            serde_json::from_value::<OperationKey>(wire.clone()).is_err(),
            "{wire}"
        );
    }
    let duplicate = r#"{"identity":{"scope":"stable","value":"uid"},"kind":"action","name":"first","name":"second"}"#;
    assert!(serde_json::from_str::<OperationKey>(duplicate).is_err());
}

#[test]
fn equal_identity_text_does_not_merge_local_and_stable_keys() {
    let stable = OperationKey::new(PluginUid::new("same"), OperationKind::Action, "run");
    let local = OperationKey {
        identity: OperationIdentity::Local(PluginId::new("same")),
        ..stable.clone()
    };
    assert_ne!(stable, local);
    assert_eq!(HashSet::from([stable, local]).len(), 2);
}

#[test]
fn shared_syntax_preserves_distinct_action_and_runable_languages() {
    for (name, action, runable) in [
        ("", false, false),
        ("a", true, true),
        ("A", true, false),
        ("1a", true, false),
        ("_a", true, false),
        ("-a", false, false),
        ("a-b", true, false),
        ("a_1", true, true),
        ("a b", false, false),
        ("é", false, false),
    ] {
        assert_eq!(is_valid_action_id(name), action, "action {name:?}");
        assert_eq!(is_valid_runable_name(name), runable, "runable {name:?}");
    }
    assert!(is_valid_action_id(&"a".repeat(64)));
    assert!(!is_valid_action_id(&"a".repeat(65)));
    assert!(is_valid_runable_name(&"a".repeat(65)));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn operation_key_wire_round_trip_preserves_all_value_text(
        text in any::<String>(),
        name in any::<String>(),
        stable in any::<bool>(),
        kind in prop::sample::select(vec![
            OperationKind::Action,
            OperationKind::Query,
            OperationKind::Stream,
        ]),
    ) {
        let identity = if stable {
            OperationIdentity::Stable(PluginUid::new(text))
        } else {
            OperationIdentity::Local(PluginId::new(text))
        };
        let key = OperationKey { identity, kind, name };
        let wire = serde_json::to_string(&key).unwrap();
        prop_assert_eq!(serde_json::from_str::<OperationKey>(&wire).unwrap(), key);
    }
}
