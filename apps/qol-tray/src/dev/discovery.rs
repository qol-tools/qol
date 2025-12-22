use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::config::DevConfig;

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredPlugin {
    pub id: String,
    pub name: String,
    pub path: String,
    pub already_linked: bool,
    pub installed_not_linked: bool,
}

pub fn discover_plugins(config: &DevConfig, plugins_dir: &Path) -> Vec<DiscoveredPlugin> {
    let search_paths = config.effective_search_paths();
    let plugin_dirs = find_plugin_dirs(&search_paths);

    let mut seen_paths = HashSet::new();
    let mut discovered = Vec::new();

    for dir in plugin_dirs {
        let abs_path = match dir.canonicalize() {
            Ok(p) => p,
            Err(_) => dir.to_path_buf(),
        };

        if !seen_paths.insert(abs_path) {
            continue;
        }

        if let Some(mut p) = try_parse_plugin_dir(&dir) {
            let (linked, installed) = check_install_status(plugins_dir, &p.id, &p.path);
            p.already_linked = linked;
            p.installed_not_linked = installed;
            if !p.already_linked {
                discovered.push(p);
            }
        }
    }

    discovered.sort_by(|a, b| a.name.cmp(&b.name));
    discovered
}

fn find_plugin_dirs(search_paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut plugins = Vec::new();

    for search_path in search_paths {
        if !search_path.exists() {
            continue;
        }

        let mut it = WalkDir::new(search_path)
            .max_depth(5)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                let name = e.file_name().to_string_lossy();
                !name.starts_with('.')
                    && name != "node_modules"
                    && name != "target"
                    && name != "vendor"
            });

        while let Some(entry) = it.next() {
            let Ok(entry) = entry else { continue };
            let path = entry.path();

            if path.is_dir() && path.join("plugin.toml").exists() {
                plugins.push(path.to_path_buf());
                it.skip_current_dir();
            }
        }
    }

    plugins
}

fn try_parse_plugin_dir(path: &Path) -> Option<DiscoveredPlugin> {
    if !path.is_dir() {
        return None;
    }

    let plugin_toml = path.join("plugin.toml");
    if !plugin_toml.exists() {
        return None;
    }

    let id = path.file_name()?.to_string_lossy().to_string();

    if id == "plugin-template" {
        return None;
    }

    let name = read_plugin_name(&plugin_toml).unwrap_or_else(|| id.clone());

    Some(DiscoveredPlugin {
        id,
        name,
        path: path.to_string_lossy().to_string(),
        already_linked: false,
        installed_not_linked: false,
    })
}

#[derive(Deserialize)]
struct MinimalManifest {
    plugin: MinimalPluginInfo,
}

#[derive(Deserialize)]
struct MinimalPluginInfo {
    name: String,
}

fn read_plugin_name(toml_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(toml_path).ok()?;

    if let Ok(manifest) = toml::from_str::<crate::plugins::PluginManifest>(&content) {
        return Some(manifest.plugin.name);
    }

    if let Ok(minimal) = toml::from_str::<MinimalManifest>(&content) {
        return Some(minimal.plugin.name);
    }

    None
}

fn check_install_status(plugins_dir: &Path, id: &str, target: &str) -> (bool, bool) {
    let link_path = plugins_dir.join(id);

    let Ok(meta) = std::fs::symlink_metadata(&link_path) else {
        return (false, false);
    };

    if !meta.file_type().is_symlink() {
        return (false, true);
    }

    let Ok(resolved) = std::fs::read_link(&link_path) else {
        return (false, true);
    };

    let target_path = Path::new(target);
    let is_linked_to_target = resolved == target_path
        || resolved.canonicalize().ok() == target_path.canonicalize().ok();

    (is_linked_to_target, !is_linked_to_target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_plugin_toml(dir: &Path) {
        fs::write(
            dir.join("plugin.toml"),
            r#"[plugin]
name = "Test Plugin"
description = "A test"
version = "1.0.0"

[menu]
label = "Test"
items = []
"#,
        )
        .unwrap();
    }

    #[test]
    fn finds_plugin_at_root() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("my-plugin");
        fs::create_dir(&plugin_dir).unwrap();
        create_plugin_toml(&plugin_dir);

        let found = find_plugin_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], plugin_dir);
    }

    #[test]
    fn finds_plugin_nested_in_subdirectory() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path().join("pointZ");
        let plugin_dir = parent.join("PointZerver");
        fs::create_dir_all(&plugin_dir).unwrap();
        create_plugin_toml(&plugin_dir);

        let found = find_plugin_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], plugin_dir);
    }

    #[test]
    fn finds_multiple_plugins() {
        let tmp = TempDir::new().unwrap();
        let p1 = tmp.path().join("plugin-a");
        let p2 = tmp.path().join("subdir").join("plugin-b");
        fs::create_dir_all(&p1).unwrap();
        fs::create_dir_all(&p2).unwrap();
        create_plugin_toml(&p1);
        create_plugin_toml(&p2);

        let found = find_plugin_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn skips_hidden_directories() {
        let tmp = TempDir::new().unwrap();
        let hidden = tmp.path().join(".hidden").join("plugin");
        fs::create_dir_all(&hidden).unwrap();
        create_plugin_toml(&hidden);

        let found = find_plugin_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 0);
    }

    #[test]
    fn skips_node_modules() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules").join("some-package");
        fs::create_dir_all(&nm).unwrap();
        create_plugin_toml(&nm);

        let found = find_plugin_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 0);
    }

    #[test]
    fn skips_target_directory() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target").join("debug");
        fs::create_dir_all(&target).unwrap();
        create_plugin_toml(&target);

        let found = find_plugin_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 0);
    }

    #[test]
    fn respects_max_depth() {
        let tmp = TempDir::new().unwrap();
        let deep = tmp.path().join("a").join("b").join("c").join("d").join("e").join("f");
        fs::create_dir_all(&deep).unwrap();
        create_plugin_toml(&deep);

        let found = find_plugin_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 0);
    }

    #[test]
    fn finds_plugin_at_depth_3() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("private").join("pointZ").join("PointZerver");
        fs::create_dir_all(&plugin_dir).unwrap();
        create_plugin_toml(&plugin_dir);

        let found = find_plugin_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 1, "Should find plugin at depth 3");
        assert_eq!(found[0], plugin_dir);
    }

    #[test]
    fn finds_plugin_at_depth_5() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("a").join("b").join("c").join("d").join("plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        create_plugin_toml(&plugin_dir);

        let found = find_plugin_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 1, "Should find plugin at depth 5");
    }

    #[test]
    fn search_paths_can_overlap_and_are_deduplicated() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("sub").join("plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        create_plugin_toml(&plugin_dir);

        let config = DevConfig {
            search_paths: vec![
                tmp.path().to_path_buf(),
                tmp.path().join("sub"),
            ],
        };

        let discovered = discover_plugins(&config, tmp.path());
        assert_eq!(discovered.len(), 1, "Should deduplicate plugin found from multiple search roots");
    }

    #[test]
    fn finds_plugin_with_minimal_toml() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("minimal-plugin");
        fs::create_dir(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.toml"),
            r#"[plugin]
name = "Minimal"
description = "Desc"
version = "0.1.0"
"#,
        )
        .unwrap();

        let config = DevConfig {
            search_paths: vec![tmp.path().to_path_buf()],
        };

        let discovered = discover_plugins(&config, tmp.path());
        assert_eq!(discovered.len(), 1, "Should find it even if TOML is minimal");
        assert_eq!(discovered[0].name, "Minimal");
    }
}
