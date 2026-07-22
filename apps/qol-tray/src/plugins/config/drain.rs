use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::paths;
use crate::plugins::config::PluginConfigManager;

pub fn drain_orphan_runtime_configs() -> usize {
    let Ok(plugins_dir) = paths::plugins_dir() else {
        return 0;
    };
    let manager = match PluginConfigManager::new() {
        Ok(manager) => manager,
        Err(err) => {
            log::warn!("[config-drain] config manager unavailable: {err:#}");
            return 0;
        }
    };

    let mut drained = 0;
    for id in installed_ids(&plugins_dir) {
        for orphan in orphan_paths(&id, &plugins_dir) {
            if drain_one(&manager, &id, &orphan) {
                drained += 1;
            }
        }
    }
    drained
}

pub fn orphan_config_paths() -> Vec<(String, PathBuf)> {
    let Ok(plugins_dir) = paths::plugins_dir() else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    for id in installed_ids(&plugins_dir) {
        for orphan in orphan_paths(&id, &plugins_dir) {
            pairs.push((id.clone(), orphan));
        }
    }
    pairs
}

fn drain_one(manager: &PluginConfigManager, id: &str, orphan: &Path) -> bool {
    let Some(orphan_config) = read_object(orphan) else {
        return false;
    };
    let host_config = manager.get_config(id).ok().flatten();
    let merged = merge_missing_keys(host_config, orphan_config);
    if let Err(err) = manager.set_config(id, merged) {
        log::warn!(
            "[config-drain] failed to fold {} into host store: {err:#}",
            orphan.display()
        );
        return false;
    }
    remove_orphan(orphan);
    log::info!(
        "[config-drain] folded orphan config {} into host store for {id}",
        orphan.display()
    );
    true
}

fn installed_ids(plugins_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect()
}

fn orphan_paths(id: &str, plugins_dir: &Path) -> Vec<PathBuf> {
    classify_orphans(&qol_config::plugin_config_paths(&[id]), plugins_dir)
}

pub(crate) fn classify_orphans(candidates: &[PathBuf], plugins_dir: &Path) -> Vec<PathBuf> {
    candidates
        .iter()
        .filter(|path| !path.starts_with(plugins_dir) && path.is_file())
        .cloned()
        .collect()
}

fn read_object(path: &Path) -> Option<Value> {
    let contents = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&contents) {
        Ok(value @ Value::Object(_)) => Some(value),
        Ok(_) => {
            log::warn!(
                "[config-drain] {} is not a config object; leaving it untouched",
                path.display()
            );
            None
        }
        Err(err) => {
            log::warn!(
                "[config-drain] {} is not valid JSON ({err}); leaving it untouched",
                path.display()
            );
            None
        }
    }
}

fn merge_missing_keys(host: Option<Value>, orphan: Value) -> Value {
    let Some(Value::Object(mut base)) = host else {
        return orphan;
    };
    if let Value::Object(extra) = orphan {
        for (key, value) in extra {
            base.entry(key).or_insert(value);
        }
    }
    Value::Object(base)
}

fn remove_orphan(path: &Path) {
    if let Err(err) = std::fs::remove_file(path) {
        log::warn!(
            "[config-drain] could not remove orphan {}: {err}",
            path.display()
        );
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_missing_keys_preserves_host_and_fills_gaps() {
        let cases = [
            (
                Some(json!({ "live_color_hex": "#fff" })),
                json!({ "live_color_hex": "#000", "devices": { "0x1": 1 } }),
                json!({ "live_color_hex": "#fff", "devices": { "0x1": 1 } }),
            ),
            (
                None,
                json!({ "devices": { "0x1": 1 } }),
                json!({ "devices": { "0x1": 1 } }),
            ),
            (
                Some(json!({ "a": 1 })),
                json!({ "a": 2, "b": 3 }),
                json!({ "a": 1, "b": 3 }),
            ),
        ];
        for (host, orphan, expected) in cases {
            assert_eq!(
                merge_missing_keys(host.clone(), orphan.clone()),
                expected,
                "host={host:?} orphan={orphan:?}",
            );
        }
    }
}
