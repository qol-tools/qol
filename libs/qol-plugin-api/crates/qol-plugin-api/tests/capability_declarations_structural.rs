//! Structural invariants for the restore-rule / pane-fields / launcher-provider
//! capability declarations.
//!
//! These tests pin the shape of capability entries that plugins write into
//! `plugin.toml`. The capability registry on qol-plugin-api is the single
//! source of truth for what fields are accepted; runtime enforcement in
//! qol-runtime relies on parsing through these types.
//!
//! Refs: workspace/docs/superpowers/specs/2026-05-12-terminal-workspace-restore-design.md
//! Closes: CONTRACT-2.

use qol_plugin_api::capability::{LauncherProviderCapability, PaneField, RestoreRuleCapability};

#[test]
fn restore_rule_capability_requires_at_least_one_template() {
    // A plugin advertising `restore-rule = true` but listing zero templates
    // it can claim is a malformed manifest: the broker would dispatch to
    // it on every pane and the plugin would always 204. Require
    // `templates` to be present and non-empty.
    let with_empty: Result<RestoreRuleCapability, _> = toml::from_str(r#"templates = []"#);
    assert!(
        with_empty.is_err(),
        "RestoreRuleCapability accepted an empty templates list; \
         the manifest parser must reject zero-template restore-rule decls"
    );

    let missing: Result<RestoreRuleCapability, _> = toml::from_str("");
    assert!(
        missing.is_err(),
        "RestoreRuleCapability accepted a missing templates field; \
         `templates` must be required"
    );
}

#[test]
fn restore_rule_capability_rejects_unknown_fields() {
    let forged = r#"
        templates = ["claude-session"]
        program = "rm"
    "#;
    let result: Result<RestoreRuleCapability, _> = toml::from_str(forged);
    assert!(
        result.is_err(),
        "RestoreRuleCapability accepted an unknown `program` field. \
         The struct is missing #[serde(deny_unknown_fields)], which is \
         the line-of-defense against future fields silently extending \
         the contract."
    );
}

#[test]
fn restore_rule_capability_round_trips_declared_fields() {
    let toml_src = r#"
        templates = ["claude-session", "psql-session"]
        pane-fields = ["foreground.exe", "foreground.pid", "foreground.cwd"]
    "#;
    let parsed: RestoreRuleCapability =
        toml::from_str(toml_src).expect("valid restore-rule capability");
    assert_eq!(parsed.templates, vec!["claude-session", "psql-session"]);
    assert_eq!(
        parsed.pane_fields,
        vec![
            PaneField::ForegroundExe,
            PaneField::ForegroundPid,
            PaneField::ForegroundCwd,
        ]
    );
}

#[test]
fn pane_field_enum_is_closed() {
    // The PaneField enum names every field a plugin may request via the
    // pane-fields capability. Unknown variants from a manifest must fail
    // to parse, not silently become a wildcard.
    let unknown: Result<PaneField, _> =
        serde_json::from_value(serde_json::Value::String("foreground.uid".into()));
    assert!(
        unknown.is_err(),
        "PaneField parsed an unknown variant `foreground.uid`. \
         Add it to the enum if intentional; otherwise the closed-set \
         guarantee is broken."
    );
}

#[test]
fn pane_field_serializes_to_dotted_lowercase() {
    // The TOML keys are dotted lowercase (foreground.exe). The serde
    // representation must match so manifest parsing and capability
    // negotiation use the same wire vocabulary.
    let v = serde_json::to_value(PaneField::ForegroundCwd).unwrap();
    assert_eq!(v, serde_json::Value::String("foreground.cwd".into()));
    let v = serde_json::to_value(PaneField::Title).unwrap();
    assert_eq!(v, serde_json::Value::String("title".into()));
}

#[test]
fn launcher_provider_capability_is_a_marker() {
    // launcher-provider takes no parameters: presence alone enables the
    // capability. Extra fields are a malformed manifest.
    let bare: LauncherProviderCapability = toml::from_str("").expect("bare marker parses");
    let _ = bare;
    let with_extra: Result<LauncherProviderCapability, _> =
        toml::from_str(r#"endpoint = "/launcher-entries""#);
    assert!(
        with_extra.is_err(),
        "LauncherProviderCapability accepted an unknown `endpoint` field. \
         The marker capability takes no parameters."
    );
}
