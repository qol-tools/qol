//! Pin the `RestoreClaim` builder contract.
//!
//! The plugin's whole job is to emit one `RestoreClaim` per matched pane.
//! The claim must:
//!
//! - Target the fixed template id `claude-session`.
//! - Carry the resolved session uuid under the `session_id` param.
//! - Carry no extra params (only `session_id` is declared).
//! - Carry an empty env (no variables leak from the plugin to the launch).
//! - Refuse to build if the jsonl basename is not a syntactically valid uuid.
//!
//! These invariants come from the design spec, section
//! "plugin-claude-sessions design", and from the security plan card 08
//! (process spoofing): the uuid is the only attacker-influenceable value
//! and is regex-bounded at the resolving end. We pin the shape here so
//! the regex on plugin-kitty has nothing extra to validate against.
//!
//! Closes: CSESS-1.3 (RestoreClaim builder invariant).

use std::path::Path;

use plugin_claude_sessions::build_claim;
use plugin_claude_sessions::claim::TEMPLATE_ID;

const VALID_UUID: &str = "01133a6e-b505-4b9d-9184-a657775b46da";

fn fixture_jsonl_path() -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "/Users/kaho/.claude/projects/-Users-kaho-repos-private-qol-tools-workspace/{VALID_UUID}.jsonl"
    ))
}

#[test]
fn builder_emits_claude_session_template_id() {
    let claim = build_claim(&fixture_jsonl_path()).expect("valid jsonl path builds a claim");
    assert_eq!(claim.template_id, TEMPLATE_ID);
    assert_eq!(claim.template_id, "claude-session");
}

#[test]
fn builder_extracts_uuid_into_session_id_param() {
    let claim = build_claim(&fixture_jsonl_path()).expect("valid jsonl path builds a claim");
    assert_eq!(
        claim.params.get("session_id").map(String::as_str),
        Some(VALID_UUID),
        "session_id param must carry the basename uuid; \
         this is the only value plugin-kitty's template will substitute"
    );
}

#[test]
fn builder_carries_only_session_id_param() {
    let claim = build_claim(&fixture_jsonl_path()).expect("valid jsonl path builds a claim");
    let keys: Vec<&str> = claim.params.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec!["session_id"],
        "params must contain exactly `session_id`; any other key is \
         an unannounced contract extension and will fail plugin-kitty's \
         strict param-schema validation"
    );
}

#[test]
fn builder_emits_empty_env() {
    let claim = build_claim(&fixture_jsonl_path()).expect("valid jsonl path builds a claim");
    assert!(
        claim.env.is_empty(),
        "RestoreClaim.env must be empty; the plugin has no reason to \
         push environment to the relaunch, and the allowlist on the \
         resolving end is the only legitimate source of env vars"
    );
}

#[test]
fn builder_rejects_jsonl_without_uuid_basename() {
    let bad = Path::new("/Users/kaho/.claude/projects/-Users-kaho/not-a-uuid.jsonl");
    let res = build_claim(bad);
    assert!(
        res.is_err(),
        "build_claim must reject a jsonl whose basename is not a \
         syntactically valid uuid; got Ok({res:?})"
    );
}

#[test]
fn builder_rejects_extension_mismatch() {
    let bad = Path::new(&format!(
        "/Users/kaho/.claude/projects/-Users-kaho/{VALID_UUID}.txt"
    ))
    .to_path_buf();
    let res = build_claim(&bad);
    assert!(
        res.is_err(),
        "build_claim must reject paths whose extension is not .jsonl"
    );
}
