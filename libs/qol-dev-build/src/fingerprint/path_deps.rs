use std::path::{Path, PathBuf};

use super::inputs::{fingerprint_inputs, FingerprintInput};

pub(super) fn collect_path_dep_inputs(plugin_path: &Path) -> Vec<Vec<FingerprintInput>> {
    resolve_path_deps(plugin_path)
        .into_iter()
        .filter_map(|(name, dep_path)| prefixed_inputs(&name, &dep_path))
        .collect()
}

fn prefixed_inputs(dep_name: &str, dep_path: &Path) -> Option<Vec<FingerprintInput>> {
    let inputs = fingerprint_inputs(dep_path).ok()?;
    let prefix = PathBuf::from(format!("__dep__/{}", dep_name));
    Some(
        inputs
            .into_iter()
            .map(|(rel, abs)| (prefix.join(rel), abs))
            .collect(),
    )
}

fn resolve_path_deps(plugin_path: &Path) -> Vec<(String, PathBuf)> {
    let cargo_toml = plugin_path.join("Cargo.toml");
    let content = match std::fs::read_to_string(&cargo_toml) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    extract_path_deps(&table, plugin_path)
}

fn extract_path_deps(table: &toml::Table, plugin_path: &Path) -> Vec<(String, PathBuf)> {
    let mut deps = Vec::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(toml::Value::Table(entries)) = table.get(section) else {
            continue;
        };
        for (name, value) in entries {
            let Some(path) = dep_path(value) else {
                continue;
            };
            let resolved = plugin_path.join(path);
            if resolved.is_dir() {
                deps.push((name.clone(), resolved));
            }
        }
    }
    deps
}

fn dep_path(value: &toml::Value) -> Option<&str> {
    let toml::Value::Table(t) = value else {
        return None;
    };
    t.get("path")?.as_str()
}
