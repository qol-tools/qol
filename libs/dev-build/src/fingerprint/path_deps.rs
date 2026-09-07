use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::hash::{read_inputs, FingerprintContent};
use super::inputs::fingerprint_inputs;
use super::FingerprintCache;

pub(super) fn collect_path_dep_contents(
    plugin_path: &Path,
    cache: &mut FingerprintCache,
) -> Vec<Vec<FingerprintContent>> {
    resolve_path_deps(plugin_path)
        .into_iter()
        .filter_map(|(name, dep_path)| prefixed_contents(&name, &dep_path, cache))
        .collect()
}

fn prefixed_contents(
    dep_name: &str,
    dep_path: &Path,
    cache: &mut FingerprintCache,
) -> Option<Vec<FingerprintContent>> {
    let inputs = match cache.dependency_contents.get(dep_path) {
        Some(inputs) => inputs.clone(),
        None => {
            let inputs = fingerprint_inputs(dep_path).ok()?;
            let contents = read_inputs(inputs).ok()?;
            cache
                .dependency_contents
                .insert(dep_path.to_path_buf(), contents.clone());
            #[cfg(test)]
            {
                cache.dependency_reads += 1;
            }
            contents
        }
    };
    let prefix = PathBuf::from(format!("__dep__/{}", dep_name));
    Some(
        inputs
            .into_iter()
            .map(|(rel, contents)| (prefix.join(rel), contents))
            .collect(),
    )
}

fn resolve_path_deps(plugin_path: &Path) -> Vec<(String, PathBuf)> {
    let Some(table) = cargo_manifest(plugin_path) else {
        return Vec::new();
    };
    extract_path_deps(&table, plugin_path, &workspace_path_deps(plugin_path))
}

fn extract_path_deps(
    table: &toml::Table,
    plugin_path: &Path,
    workspace_deps: &HashMap<String, PathBuf>,
) -> Vec<(String, PathBuf)> {
    dependency_tables(table)
        .into_iter()
        .flat_map(|entries| entries.iter())
        .filter_map(|(name, value)| dependency_path(name, value, plugin_path, workspace_deps))
        .collect()
}

fn dependency_tables(table: &toml::Table) -> Vec<&toml::Table> {
    let mut tables = direct_dependency_tables(table);
    let Some(targets) = table.get("target").and_then(toml::Value::as_table) else {
        return tables;
    };
    for target in targets.values() {
        let Some(target) = target.as_table() else {
            continue;
        };
        tables.extend(direct_dependency_tables(target));
    }
    tables
}

fn direct_dependency_tables(table: &toml::Table) -> Vec<&toml::Table> {
    ["dependencies", "dev-dependencies", "build-dependencies"]
        .into_iter()
        .filter_map(|section| table.get(section).and_then(toml::Value::as_table))
        .collect()
}

fn dependency_path(
    name: &str,
    value: &toml::Value,
    plugin_path: &Path,
    workspace_deps: &HashMap<String, PathBuf>,
) -> Option<(String, PathBuf)> {
    let path = direct_path(value)
        .map(|path| plugin_path.join(path))
        .or_else(|| workspace_path(name, value, workspace_deps))?;
    path.is_dir().then(|| (name.to_string(), path))
}

fn workspace_path(
    name: &str,
    value: &toml::Value,
    workspace_deps: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    workspace_dependency(value).then(|| workspace_deps.get(name).cloned())?
}

fn workspace_dependency(value: &toml::Value) -> bool {
    value
        .as_table()
        .and_then(|table| table.get("workspace"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
}

fn workspace_path_deps(plugin_path: &Path) -> HashMap<String, PathBuf> {
    let Some((workspace_root, manifest)) = workspace_manifest(plugin_path) else {
        return HashMap::new();
    };
    let Some(workspace) = manifest.get("workspace").and_then(toml::Value::as_table) else {
        return HashMap::new();
    };
    let Some(dependencies) = workspace
        .get("dependencies")
        .and_then(toml::Value::as_table)
    else {
        return HashMap::new();
    };
    dependencies
        .iter()
        .filter_map(|(name, value)| direct_path(value).map(|path| (name, path)))
        .map(|(name, path)| (name.clone(), workspace_root.join(path)))
        .filter(|(_, path)| path.is_dir())
        .collect()
}

fn workspace_manifest(plugin_path: &Path) -> Option<(PathBuf, toml::Table)> {
    plugin_path.ancestors().find_map(|root| {
        let manifest = cargo_manifest(root)?;
        manifest
            .contains_key("workspace")
            .then(|| (root.to_path_buf(), manifest))
    })
}

fn cargo_manifest(path: &Path) -> Option<toml::Table> {
    std::fs::read_to_string(path.join("Cargo.toml"))
        .ok()?
        .parse()
        .ok()
}

fn direct_path(value: &toml::Value) -> Option<&str> {
    let toml::Value::Table(t) = value else {
        return None;
    };
    t.get("path")?.as_str()
}
