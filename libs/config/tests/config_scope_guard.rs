use std::path::{Path, PathBuf};

const SCOPES: [&str; 3] = ["core", "os", "device"];

fn plugin_dirs() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|err| panic!("read {}: {err}", root.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|dir| dir.join("qol-config.toml").is_file())
        .collect();
    dirs.sort();
    dirs
}

fn declared_scopes(plugin_dir: &Path) -> toml::value::Table {
    let path = plugin_dir.join("plugin.toml");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let manifest: toml::Value =
        toml::from_str(&content).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()));
    manifest
        .get("config")
        .and_then(|config| config.get("scope"))
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default()
}

fn stored_keys(plugin_dir: &Path) -> Vec<String> {
    let spec = qol_config::contract::parse_spec(plugin_dir.join("qol-config.toml"))
        .unwrap_or_else(|err| panic!("parse {}: {err:?}", plugin_dir.display()));
    spec.fields
        .iter()
        .filter(|(_, field)| field.kind.has_stored_value())
        .map(|(id, field)| field.config_key.clone().unwrap_or_else(|| id.clone()))
        .collect()
}

fn is_classified(key: &str, scopes: &toml::value::Table) -> bool {
    scopes
        .keys()
        .any(|declared| key == declared || key.starts_with(&format!("{declared}.")))
}

#[test]
fn every_stored_field_declares_its_sync_scope() {
    let mut missing = Vec::new();
    for dir in plugin_dirs() {
        let scopes = declared_scopes(&dir);
        let plugin = dir.file_name().unwrap().to_string_lossy().into_owned();
        for key in stored_keys(&dir) {
            if !is_classified(&key, &scopes) {
                missing.push(format!("{plugin}: {key}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "stored settings without a sync scope; add each to [config.scope] in plugin.toml as \
         \"core\" (syncs everywhere), \"os\" (syncs to the same OS) or \"device\" (never syncs, \
         for hardware addresses, device names, ports and local paths):\n{}",
        missing.join("\n")
    );
}

#[test]
fn declared_scopes_are_known_values_with_quoted_dotted_paths() {
    for dir in plugin_dirs() {
        let plugin = dir.file_name().unwrap().to_string_lossy().into_owned();
        for (key, value) in declared_scopes(&dir) {
            let scope = value.as_str().unwrap_or_else(|| {
                panic!(
                    "{plugin}: [config.scope] {key} is not a string; quote dotted paths like \"{key}.field\""
                )
            });
            assert!(
                SCOPES.contains(&scope),
                "{plugin}: [config.scope] {key} = {scope:?} must be one of {SCOPES:?}"
            );
        }
    }
}
