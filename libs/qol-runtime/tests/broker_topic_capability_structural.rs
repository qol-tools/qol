//! Structural invariants for the event-bus topic capability gate.
//!
//! The broker gates both publish and subscribe on the plugin's
//! declared topic set, mirroring the pane-field gate
//! (`broker_field_capability_structural.rs`). Unlike pane fields there
//! is no always-allowed set: nothing on the bus is implicitly public.
//! These tests pin the gate's decision table and the error echo.
//!
//! Refs: docs/ecosystem-features.md (P0-6), the broker ADR
//! (docs/adr/RUNTIME-1-af-unix-broker-with-peer-cred-auth-and-pull-pane-f.md).

use qol_runtime::broker::{check_topic_access, TopicCheckError};

const TOPIC: &str = "kitty.session_opened";

#[test]
fn declared_topic_passes_check() {
    let declared = vec![TOPIC.to_string()];
    assert!(
        check_topic_access(&declared, TOPIC).is_ok(),
        "check_topic_access denied a topic the plugin had declared; \
         the capability gate is rejecting legitimate participation"
    );
}

#[test]
fn undeclared_topic_is_denied_and_echoed() {
    let declared = vec!["other.event".to_string()];
    match check_topic_access(&declared, TOPIC) {
        Err(TopicCheckError::NotDeclared { topic }) => {
            assert_eq!(
                topic, TOPIC,
                "TopicCheckError::NotDeclared echoed the wrong topic: {topic} vs {TOPIC}"
            );
        }
        other => panic!(
            "check_topic_access({TOPIC}) with unrelated declared topics returned {other:?}; \
             expected NotDeclared"
        ),
    }
}

#[test]
fn empty_declared_set_denies_everything() {
    // Unlike the pane-field gate there is no always-allowed set: a
    // plugin with no declared topics can neither publish nor subscribe.
    let declared: Vec<String> = Vec::new();
    for topic in [TOPIC, "public.everything", "free-for-all"] {
        assert!(
            matches!(
                check_topic_access(&declared, topic),
                Err(TopicCheckError::NotDeclared { .. })
            ),
            "topic {topic} passed the gate with an empty declared set; \
             the bus must default-deny every topic"
        );
    }
}

#[test]
fn declared_set_is_exact_match_not_prefix() {
    // Declaring a namespace prefix must not grant the topic itself:
    // the gate is exact-string, mirroring the pane-field vocabulary.
    let declared = vec!["kitty".to_string(), "kitty.session".to_string()];
    assert!(
        matches!(
            check_topic_access(&declared, TOPIC),
            Err(TopicCheckError::NotDeclared { .. })
        ),
        "declaring `kitty` or `kitty.session` implicitly granted `{TOPIC}`; \
         the gate must remain exact-match, not prefix-scoped"
    );
}
