//! Pin the shape of `plugin.toml`'s capability declaration.
//!
//! plugin-kitty's broker dispatches to this plugin based on what its
//! manifest says it can claim, and what pane fields it needs to pull.
//! The shape declared here matches the design spec's
//! "plugin-claude-sessions design > Manifest" subsection. Drift between
//! this test and `plugin.toml` is a contract bug.
//!
//! The test loads `plugin.toml` from the crate root (cargo runs
//! integration tests with CWD = `CARGO_MANIFEST_DIR`), parses the
//! `[capabilities.restore-rule]` section through `qol-plugin-api`'s
//! typed `RestoreRuleCapability`, and asserts the declared values.
//!
//! Closes: CSESS-1.5 (manifest contract invariant).

use qol_plugin_api::capability::{PaneField, RestoreRuleCapability};
use serde::Deserialize;

#[derive(Deserialize)]
struct Manifest {
    capabilities: Capabilities,
}

#[derive(Deserialize)]
struct Capabilities {
    #[serde(rename = "restore-rule")]
    restore_rule: RestoreRuleCapability,
}

fn load_manifest() -> Manifest {
    let body =
        std::fs::read_to_string("plugin.toml").expect("plugin.toml must exist at the crate root");
    toml::from_str(&body)
        .expect("plugin.toml must parse with a typed [capabilities.restore-rule] block")
}

#[test]
fn manifest_declares_claude_session_template() {
    let m = load_manifest();
    assert_eq!(
        m.capabilities.restore_rule.templates,
        vec!["claude-session".to_string()],
        "manifest must declare exactly the `claude-session` template; \
         declaring more would mean the plugin can claim against templates \
         the user did not approve via this plugin's install flow"
    );
}

#[test]
fn manifest_declares_pane_fields_required_for_resolution() {
    let m = load_manifest();
    // foreground.exe -> short-circuit non-claude PIDs
    // foreground.pid -> the PID to feed libproc
    // foreground.cwd -> the cwd to compute the encoded-cwd against (sanity
    //                   cross-check against the jsonl path's middle segment)
    assert_eq!(
        m.capabilities.restore_rule.pane_fields,
        vec![
            PaneField::ForegroundExe,
            PaneField::ForegroundPid,
            PaneField::ForegroundCwd,
        ],
        "manifest must request exactly the three fields the resolver \
         needs; requesting fewer breaks resolution, requesting more \
         leaks information per security-plan card 07"
    );
}

#[test]
fn manifest_does_not_request_title_or_argv() {
    // Title and argv can carry secrets; the plugin has no reason to
    // see them, so they must not appear in the pane-fields list.
    // (Security plan card 07: default-deny info disclosure.)
    let m = load_manifest();
    for forbidden in [
        PaneField::Title,
        PaneField::ForegroundArgv,
        PaneField::ForegroundArgvFull,
        PaneField::Cwd,
    ] {
        assert!(
            !m.capabilities.restore_rule.pane_fields.contains(&forbidden),
            "manifest requested {forbidden:?}; this plugin does not need it \
             and pulling it would widen the information-disclosure surface \
             with no operational benefit"
        );
    }
}
