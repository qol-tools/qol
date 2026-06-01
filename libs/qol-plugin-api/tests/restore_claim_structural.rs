//! Structural invariants for the restore-rule contract.
//!
//! Authority over which programs may run after a workspace reboot lives in
//! plugin-kitty's user-owned template registry, never in plugin returns.
//! These tests lock the contract so that a future change cannot regress the
//! security model without a visible test failure.
//!
//! See `workspace/docs/superpowers/specs/2026-05-12-terminal-workspace-restore-design.md`
//! and `workspace/docs/superpowers/plans/2026-05-12-terminal-workspace-restore-security-plan.md`.
//! Closes: CONTRACT-1, CONTRACT-2, CONTRACT-3.

use std::collections::BTreeMap;

use qol_plugin_api::RestoreClaim;

#[test]
fn restore_claim_has_no_command_authority() {
    let claim = RestoreClaim {
        template_id: "claude-session".to_string(),
        params: BTreeMap::new(),
        env: Vec::new(),
    };
    let value = serde_json::to_value(&claim).expect("RestoreClaim must serialize");
    let object = value
        .as_object()
        .expect("RestoreClaim must serialize to a JSON object");
    let keys: Vec<&str> = object.keys().map(String::as_str).collect();

    for forbidden in ["program", "args", "argv", "cmd", "command", "exec", "bin"] {
        assert!(
            !keys.contains(&forbidden),
            "RestoreClaim leaked program authority via field `{forbidden}`: {keys:?}. \
             The restore-rule contract MUST NOT let a plugin communicate a program \
             or argv; authority over what runs lives in plugin-kitty's template registry."
        );
    }
}

#[test]
fn restore_claim_rejects_command_fields_on_wire() {
    // A malicious or buggy plugin could attempt to inject a program field
    // through the wire payload. The contract refuses such payloads via
    // serde's `deny_unknown_fields`.
    let malicious = r#"{"template_id":"t","params":{},"program":"rm"}"#;
    let result: Result<RestoreClaim, _> = serde_json::from_str(malicious);
    assert!(
        result.is_err(),
        "RestoreClaim accepted a forged `program` field on the wire. \
         The struct is missing `#[serde(deny_unknown_fields)]`, which is the \
         line-of-defense that enforces the structural invariant at parse time."
    );
}

#[test]
fn restore_claim_round_trip_preserves_only_declared_fields() {
    let claim = RestoreClaim {
        template_id: "claude-session".to_string(),
        params: BTreeMap::from([("uuid".to_string(), "abc-123".to_string())]),
        env: Vec::new(),
    };
    let json = serde_json::to_string(&claim).expect("serialize");
    let back: RestoreClaim = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.template_id, claim.template_id);
    assert_eq!(back.params, claim.params);
    assert_eq!(back.env, claim.env);
}
