use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct LinkedPlugin {
    pub id: String,
    pub name: String,
    pub source: String,
    pub has_cargo: bool,
    pub needs_rebuild: bool,
    pub rebuild_reason: String,
    pub fingerprint: Option<String>,
    pub last_built_fingerprint: Option<String>,
    pub logs_muted: bool,
    pub suppressed_log_patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinkRequest {
    pub path: String,
}

pub fn load_dev_links(config_dir: &Path) -> HashMap<String, PathBuf> {
    let path = config_dir.join("dev-links.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn save_dev_links(config_dir: &Path, links: &HashMap<String, PathBuf>) -> Result<(), String> {
    let path = config_dir.join("dev-links.json");
    let tmp_path = config_dir.join(".dev-links.json.tmp");
    let content = serde_json::to_string_pretty(links)
        .map_err(|e| format!("Failed to serialize dev-links: {}", e))?;
    std::fs::write(&tmp_path, &content)
        .map_err(|e| format!("Failed to write dev-links temp file: {}", e))?;
    std::fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Failed to finalize dev-links.json: {}", e))
}

pub fn list_linked_plugins(config_dir: &Path) -> Result<Vec<LinkedPlugin>, String> {
    let links = load_dev_links(config_dir);
    let known_fingerprints = super::build::load_build_fingerprints(config_dir);
    let log_controls = crate::plugins::log_control::load_all_controls(config_dir);
    let plans = super::build::plan_linked_plugin_builds(&links, &known_fingerprints);
    let mut plans_by_id = HashMap::new();
    for plan in plans {
        plans_by_id.insert(plan.plugin_id.clone(), plan);
    }

    let mut plugins: Vec<LinkedPlugin> = links
        .iter()
        .map(|(id, path)| {
            let name = read_plugin_name(&path.join("plugin.toml")).unwrap_or_else(|| id.clone());
            let plan = plans_by_id.get(id);
            let log_control = log_controls.get(id).cloned().unwrap_or_default();
            LinkedPlugin {
                id: id.clone(),
                name,
                source: path.to_string_lossy().to_string(),
                has_cargo: plan.map(|p| p.has_cargo).unwrap_or(false),
                needs_rebuild: plan.map(|p| p.needs_rebuild).unwrap_or(false),
                rebuild_reason: plan
                    .map(|p| p.reason.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                fingerprint: plan.and_then(|p| p.current_fingerprint.clone()),
                last_built_fingerprint: plan.and_then(|p| p.last_built_fingerprint.clone()),
                logs_muted: log_control.muted,
                suppressed_log_patterns: log_control.suppress_patterns,
            }
        })
        .collect();

    plugins.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(plugins)
}

pub fn create_link(source: &Path, config_dir: &Path) -> Result<String, String> {
    if !source.exists() {
        return Err("Source path does not exist".to_string());
    }

    if !source.join("plugin.toml").exists() {
        return Err("No plugin.toml found in source".to_string());
    }

    let plugin_id = source
        .file_name()
        .ok_or("Invalid path")?
        .to_string_lossy()
        .to_string();

    let mut links = load_dev_links(config_dir);

    if links.contains_key(&plugin_id) {
        return Err("Already linked".to_string());
    }

    links.insert(plugin_id.clone(), source.to_path_buf());
    save_dev_links(config_dir, &links)?;

    log::info!("Created dev-link: {} -> {:?}", plugin_id, source);
    Ok(plugin_id)
}

pub fn remove_link(id: &str, config_dir: &Path) -> Result<(), String> {
    let mut links = load_dev_links(config_dir);

    if links.remove(id).is_none() {
        return Err("Plugin not dev-linked".to_string());
    }

    save_dev_links(config_dir, &links)?;
    log::info!("Removed dev-link: {}", id);
    Ok(())
}

fn read_plugin_name(toml_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(toml_path).ok()?;
    let manifest: crate::plugins::PluginManifest = toml::from_str(&content).ok()?;
    Some(manifest.plugin.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_plugin_toml(dir: &Path, name: &str) {
        fs::write(
            dir.join("plugin.toml"),
            format!(
                "[plugin]\nname = \"{}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n",
                name
            ),
        )
        .unwrap();
    }

    #[test]
    fn load_dev_links_returns_empty_when_no_file() {
        let tmp = TempDir::new().unwrap();
        assert!(load_dev_links(tmp.path()).is_empty());
    }

    #[test]
    fn create_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("my-plugin");
        fs::create_dir(&source).unwrap();
        write_plugin_toml(&source, "My Plugin");

        let id = create_link(&source, tmp.path()).unwrap();
        assert_eq!(id, "my-plugin");

        let links = load_dev_links(tmp.path());
        assert_eq!(links.len(), 1);
        assert_eq!(links["my-plugin"], source);
    }

    #[test]
    fn create_link_rejects_duplicate() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("foo");
        fs::create_dir(&source).unwrap();
        write_plugin_toml(&source, "Foo");

        create_link(&source, tmp.path()).unwrap();
        let err = create_link(&source, tmp.path()).unwrap_err();
        assert!(err.contains("Already linked"));
    }

    #[test]
    fn create_link_rejects_missing_source() {
        let tmp = TempDir::new().unwrap();
        let err = create_link(Path::new("/nonexistent"), tmp.path()).unwrap_err();
        assert!(err.contains("does not exist"));
    }

    #[test]
    fn create_link_rejects_missing_toml() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("no-toml");
        fs::create_dir(&source).unwrap();

        let err = create_link(&source, tmp.path()).unwrap_err();
        assert!(err.contains("No plugin.toml"));
    }

    #[test]
    fn remove_link_removes_entry() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("foo");
        fs::create_dir(&source).unwrap();
        write_plugin_toml(&source, "Foo");

        create_link(&source, tmp.path()).unwrap();
        remove_link("foo", tmp.path()).unwrap();

        assert!(load_dev_links(tmp.path()).is_empty());
    }

    #[test]
    fn remove_link_rejects_unknown_id() {
        let tmp = TempDir::new().unwrap();
        let err = remove_link("nonexistent", tmp.path()).unwrap_err();
        assert!(err.contains("not dev-linked"));
    }

    #[test]
    fn list_linked_plugins_enriches_with_name() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("foo");
        fs::create_dir(&source).unwrap();
        write_plugin_toml(&source, "Fancy Plugin");

        create_link(&source, tmp.path()).unwrap();
        let listed = list_linked_plugins(tmp.path()).unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "foo");
        assert_eq!(listed[0].name, "Fancy Plugin");
        assert_eq!(listed[0].source, source.to_string_lossy());
        assert!(!listed[0].has_cargo);
        assert_eq!(listed[0].rebuild_reason, "Cargo.toml missing");
    }
}
