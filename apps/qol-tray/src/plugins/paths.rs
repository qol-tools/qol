use anyhow::Result;
use std::path::{Path, PathBuf};

const CONFIG_FILE_NAME: &str = "config.json";
const CONFIG_CONTRACT_FILE_NAME: &str = "qol-config.toml";
const RUNABLE_CONTRACT_FILE_NAME: &str = "qol-runtime.toml";

pub(crate) fn resolve_plugin_root(plugin_id: &str) -> Result<PathBuf> {
    if !crate::paths::is_safe_path_component(plugin_id) {
        anyhow::bail!("Invalid plugin ID: {}", plugin_id);
    }
    let plugins_dir = crate::paths::plugins_dir()?;
    Ok(resolve_plugin_root_from_plugins_dir(
        &plugins_dir,
        plugin_id,
    ))
}

pub(crate) fn resolve_plugin_root_from_plugins_dir(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    if let Some(active) = crate::plugins::registry::current_active_path(plugin_id) {
        let overridden = worktree_override_root(&active);
        return overridden.unwrap_or(active);
    }
    plugins_dir.join(plugin_id)
}

#[cfg(feature = "dev")]
fn worktree_override_root(dev_link: &Path) -> Option<PathBuf> {
    let config_dir = crate::paths::shared_config_dir().ok()?;
    let branch = crate::dev::get_active_worktree_branch(&config_dir)?;
    let mut map = std::collections::HashMap::new();
    map.insert("p".to_string(), dev_link.to_path_buf());
    let resolved = crate::dev::resolve_worktree_paths(&map, Some(&branch));
    resolved.into_values().next().filter(|p| p != dev_link)
}

#[cfg(not(feature = "dev"))]
fn worktree_override_root(_dev_link: &Path) -> Option<PathBuf> {
    None
}

pub(crate) fn config_path(plugin_root: &Path) -> PathBuf {
    plugin_root.join(CONFIG_FILE_NAME)
}

pub(crate) fn config_contract_path(plugin_root: &Path) -> PathBuf {
    plugin_root.join(CONFIG_CONTRACT_FILE_NAME)
}

pub(crate) fn runable_contract_path(plugin_root: &Path) -> PathBuf {
    plugin_root.join(RUNABLE_CONTRACT_FILE_NAME)
}

pub(crate) fn has_custom_ui(plugin_root: &Path) -> bool {
    is_real_custom_ui(&plugin_root.join("ui/index.html"))
}

fn is_real_custom_ui(index_path: &Path) -> bool {
    let content = match std::fs::read_to_string(index_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    !content.contains("initAutoConfigPage")
}

pub(crate) fn has_config(plugin_root: &Path) -> bool {
    config_contract_path(plugin_root).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_plugin_ui(root: &Path, html: &str) {
        let ui_dir = root.join("ui");
        std::fs::create_dir_all(&ui_dir).unwrap();
        std::fs::write(ui_dir.join("index.html"), html).unwrap();
    }

    #[test]
    fn has_custom_ui_rejects_auto_config_template() {
        let dir = tempfile::tempdir().unwrap();

        let auto_config_html = r#"<script type="module">
import { initAutoConfigPage } from '/auto-config-bootstrap.js';
initAutoConfigPage();
</script>"#;

        let real_ui_html = r#"<!DOCTYPE html>
<html><head><title>My Plugin</title></head>
<body><script type="module" src="./app.js"></script></body>
</html>"#;

        let cases = [
            ("auto-config template", auto_config_html, false),
            ("real custom UI", real_ui_html, true),
        ];

        for (label, html, expected) in cases {
            let plugin_root = dir.path().join(label);
            write_plugin_ui(&plugin_root, html);
            assert_eq!(has_custom_ui(&plugin_root), expected, "case: {label}");
        }
    }

    #[test]
    fn has_custom_ui_false_when_no_ui_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_custom_ui(dir.path()));
    }

    #[test]
    fn runable_contract_path_joins_filename() {
        let root = Path::new("/tmp/plugin-foo");
        assert_eq!(
            runable_contract_path(root),
            Path::new("/tmp/plugin-foo/qol-runtime.toml")
        );
    }
}
