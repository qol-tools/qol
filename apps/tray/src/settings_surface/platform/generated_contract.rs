use crate::plugins::manifest::{ActionType, DeclaredAction, PluginManifest, ShortcutDeclaration};

pub(super) fn rail_label(manifest: &PluginManifest) -> String {
    let menu_label = manifest.menu.label.trim();
    if !menu_label.is_empty() {
        return menu_label.to_string();
    }
    manifest.plugin.name.trim().to_string()
}

pub(super) fn generate_contract(manifest: &PluginManifest) -> String {
    let title = rail_label(manifest);
    let description = manifest.plugin.description.trim();
    let actions: Vec<DeclaredAction> = manifest
        .executable_actions()
        .into_iter()
        .filter(|action| action.kind != ActionType::Settings)
        .collect();
    let shortcuts: &[ShortcutDeclaration] = &manifest.shortcuts;

    let mut out = String::new();
    out.push_str("schema_version = 1\n");
    push_pair(&mut out, "title", &title);
    if !description.is_empty() {
        push_pair(&mut out, "description", description);
    }
    if !actions.is_empty() {
        out.push_str("\n[section.actions]\n");
        out.push_str("label = \"Actions\"\n");
        for action in &actions {
            push_action_field(&mut out, &action.id, &action.label, "actions", &action.id);
        }
    }
    if !shortcuts.is_empty() {
        out.push_str("\n[section.shortcuts]\n");
        out.push_str("label = \"Shortcuts\"\n");
        for shortcut in shortcuts {
            let field_id = format!("shortcut__{}", shortcut.id);
            push_action_field(
                &mut out,
                &field_id,
                &shortcut.name,
                "shortcuts",
                &shortcut.action,
            );
        }
    }
    out
}

fn push_pair(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(" = ");
    out.push_str(&toml_string(value));
    out.push('\n');
}

fn push_action_field(out: &mut String, field_id: &str, label: &str, section: &str, action: &str) {
    out.push_str("\n[field.");
    out.push_str(field_id);
    out.push_str("]\n");
    out.push_str("type = \"action\"\n");
    push_pair(out, "label", label);
    push_pair(out, "section", section);
    push_pair(out, "action", action);
    out.push_str("variant = \"primary\"\n");
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_config::contract::{parse_spec_str, FieldKind};

    fn parse_manifest(toml_str: &str) -> PluginManifest {
        toml::from_str(toml_str).expect("manifest parse")
    }

    const SESSION_MANIFEST: &str = r#"[plugin]
id = "qol-session"
name = "Session"
description = "Lock the screen from a hotkey on every platform"
version = "0.1.0"

[runtime]
command = "qol-session"

[action.lock]
label = "Lock Screen"
args = ["lock"]

[menu]
label = "Session"
items = []

[[shortcuts]]
id = "lock"
name = "Lock Screen"
action = "lock"
"#;

    #[test]
    fn rail_label_prefers_menu_label_then_falls_back_to_plugin_name() {
        let cases = [
            (
                r#"[plugin]
id = "x"
name = "Fallback"
description = ""
version = "1.0.0"
[menu]
label = "Menu Label"
items = []
"#,
                "Menu Label",
            ),
            (
                r#"[plugin]
id = "x"
name = "Fallback"
description = ""
version = "1.0.0"
[menu]
label = ""
items = []
"#,
                "Fallback",
            ),
            (
                r#"[plugin]
id = "x"
name = "Fallback"
description = ""
version = "1.0.0"
[menu]
label = "   "
items = []
"#,
                "Fallback",
            ),
        ];
        for (manifest_toml, expected) in cases {
            let manifest = parse_manifest(manifest_toml);
            assert_eq!(rail_label(&manifest), expected, "manifest: {manifest_toml}");
        }
    }

    #[test]
    fn generated_contract_parses_and_lists_each_non_settings_action() {
        let manifest = parse_manifest(
            r#"[plugin]
id = "qol-example"
name = "Example"
description = "An example plugin."
version = "0.1.0"
[runtime]
command = "qol-example"
[action.run]
label = "Run"
args = ["run"]
[action.other]
label = "Other"
args = ["other"]
[action.settings]
label = "Settings"
kind = "settings"
args = ["settings"]
[menu]
label = "Example"
items = []
"#,
        );
        let generated = generate_contract(&manifest);
        let spec = parse_spec_str(&generated).expect("generated contract parses");
        assert_eq!(spec.title.as_deref(), Some("Example"));
        assert_eq!(spec.description.as_deref(), Some("An example plugin."));
        let run_field = spec.fields.get("run").expect("run action field");
        assert_eq!(run_field.kind, FieldKind::Action);
        assert_eq!(run_field.label.as_deref(), Some("Run"));
        assert_eq!(run_field.action.as_deref(), Some("run"));
        assert_eq!(run_field.section.as_deref(), Some("actions"));
        let other_field = spec.fields.get("other").expect("other action field");
        assert_eq!(other_field.action.as_deref(), Some("other"));
        assert!(
            spec.fields.get("settings").is_none(),
            "settings action is not in the generated page"
        );
        assert!(spec.section("actions").is_some());
    }

    #[test]
    fn generated_contract_renders_declared_shortcuts_in_their_own_section() {
        let manifest = parse_manifest(SESSION_MANIFEST);
        let generated = generate_contract(&manifest);
        let spec = parse_spec_str(&generated).expect("generated contract parses");
        assert_eq!(spec.title.as_deref(), Some("Session"));
        let lock = spec.fields.get("lock").expect("action lock");
        assert_eq!(lock.section.as_deref(), Some("actions"));
        assert_eq!(lock.action.as_deref(), Some("lock"));
        let shortcut = spec
            .fields
            .get("shortcut__lock")
            .expect("shortcut field is prefixed to avoid collision");
        assert_eq!(shortcut.kind, FieldKind::Action);
        assert_eq!(shortcut.section.as_deref(), Some("shortcuts"));
        assert_eq!(shortcut.action.as_deref(), Some("lock"));
        assert_eq!(shortcut.label.as_deref(), Some("Lock Screen"));
    }

    #[test]
    fn generated_contract_renders_without_actions_or_shortcuts() {
        let manifest = parse_manifest(
            r#"[plugin]
id = "qol-bare"
name = "Bare"
description = ""
version = "0.1.0"
[menu]
label = "Bare"
items = []
"#,
        );
        let generated = generate_contract(&manifest);
        let spec = parse_spec_str(&generated).expect("generated contract parses");
        assert_eq!(spec.title.as_deref(), Some("Bare"));
        assert!(spec.description.is_none());
        assert!(spec.fields.is_empty());
        assert!(spec.sections.is_empty());
    }

    #[test]
    fn generated_contract_quotes_special_characters_safely() {
        let manifest = parse_manifest(
            "[plugin]\n\
             id = \"qol-escaped\"\n\
             name = \"Quote \\\"Me\\\"\"\n\
             description = \"Line1\\nLine2\"\n\
             version = \"0.1.0\"\n\
             [action.run]\n\
             label = \"Say \\\"Hi\\\"\"\n\
             args = [\"run\"]\n\
             [menu]\n\
             label = \"Quote \\\"Me\\\"\"\n\
             items = []\n",
        );
        let generated = generate_contract(&manifest);
        let spec = parse_spec_str(&generated).expect("escapes round-trip through TOML");
        assert_eq!(spec.title.as_deref(), Some("Quote \"Me\""));
        assert_eq!(spec.description.as_deref(), Some("Line1\nLine2"));
        let run = spec.fields.get("run").expect("run field");
        assert_eq!(run.label.as_deref(), Some("Say \"Hi\""));
    }
}
