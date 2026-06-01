use std::fs;
use std::path::PathBuf;

fn manifest_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn version_in(file: &str, table: &str) -> String {
    let path = manifest_root().join(file);
    let body =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    let parsed: toml::Value =
        toml::from_str(&body).unwrap_or_else(|e| panic!("parse {}: {}", path.display(), e));
    parsed
        .get(table)
        .and_then(|t| t.get("version"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("missing [{table}].version in {}", path.display()))
}

#[test]
fn cargo_and_plugin_manifests_agree_on_version() {
    let cargo = version_in("Cargo.toml", "package");
    let plugin = version_in("plugin.toml", "plugin");
    assert_eq!(
        cargo, plugin,
        "Cargo.toml [package].version ({cargo}) must match plugin.toml [plugin].version ({plugin}). \
         Bump both together so plugin-version CI stays green."
    );
}
