use std::path::Path;

fn plugin_config(plugin_id: &str) -> toml::Value {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir
        .join("../../plugins")
        .join(plugin_id)
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
    let launcher = plugin_config("plugin-launcher");
    let alt_tab = plugin_config("plugin-alt-tab");

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
