use std::path::Path;

fn plugin_config(source_directory: &str) -> toml::Value {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir
        .join("../../plugins")
        .join(source_directory)
        .join("qol-config.toml");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    toml::from_str(&content).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

fn field<'a>(config: &'a toml::Value, name: &str) -> &'a toml::value::Table {
    config
        .get("field")
        .and_then(|fields| fields.get(name))
        .and_then(toml::Value::as_table)
        .unwrap_or_else(|| panic!("missing [field.{name}]"))
}

fn assert_shared_keys_match(
    launcher: &toml::value::Table,
    alt_tab: &toml::value::Table,
    keys: &[&str],
) {
    for key in keys {
        assert_eq!(
            launcher.get(*key),
            alt_tab.get(*key),
            "ghost field key {key:?} drifted between launcher and alt-tab"
        );
    }
}

#[test]
fn ghost_debug_fields_keep_their_shared_schema_contract() {
    let launcher = plugin_config("launcher");
    let alt_tab = plugin_config("alt-tab");

    assert_shared_keys_match(
        field(&launcher, "display_ghost_opacity"),
        field(&alt_tab, "display_ghost_opacity"),
        &[
            "type",
            "config_key",
            "label",
            "default",
            "min",
            "max",
            "step",
        ],
    );
    assert_shared_keys_match(
        field(&launcher, "display_ghost_debug_color"),
        field(&alt_tab, "display_ghost_debug_color"),
        &["type", "config_key", "label"],
    );
}

#[test]
fn ghost_debug_sections_are_dev_only() {
    for plugin in ["launcher", "alt-tab"] {
        let config = plugin_config(plugin);
        let section = field(&config, "display_ghost_opacity")
            .get("section")
            .and_then(toml::Value::as_str)
            .unwrap_or_else(|| panic!("{plugin}: ghost opacity has no section"));
        let dev_only = config
            .get("section")
            .and_then(|sections| sections.get(section))
            .and_then(|section| section.get("dev_only"))
            .and_then(toml::Value::as_bool);
        assert_eq!(
            dev_only,
            Some(true),
            "{plugin}: [section.{section}] must be dev_only"
        );
    }
}
