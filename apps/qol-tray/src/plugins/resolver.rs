use crate::file_io;
use crate::plugins::PluginId;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    pub id: PluginId,
    pub path: PathBuf,
    pub source: PluginSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginSource {
    Installed,
    DevLinked,
}

impl PluginSource {
    pub fn is_live_source(&self) -> bool {
        match self {
            Self::DevLinked => true,
            Self::Installed => false,
        }
    }
}

pub fn resolve_all(
    plugins_dir: &Path,
    dev_links: &HashMap<String, PathBuf>,
) -> Vec<ResolvedPlugin> {
    let dev_link_targets: HashSet<PathBuf> = dev_links
        .values()
        .map(|p| file_io::canonical_or_original(p))
        .collect();
    let mut resolved = scan_installed(plugins_dir, &dev_link_targets);
    apply_dev_links(&mut resolved, dev_links);
    let mut result: Vec<_> = resolved.into_values().collect();
    result.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    result
}

fn scan_installed(
    plugins_dir: &Path,
    dev_link_targets: &HashSet<PathBuf>,
) -> HashMap<PluginId, ResolvedPlugin> {
    let mut resolved = HashMap::new();
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return resolved;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let raw_id = entry.file_name().to_string_lossy().into_owned();
        if should_skip(&raw_id, &path, dev_link_targets) {
            continue;
        }
        let id = PluginId::new(raw_id);
        resolved.insert(
            id.clone(),
            ResolvedPlugin {
                id,
                path,
                source: PluginSource::Installed,
            },
        );
    }
    resolved
}

fn should_skip(id: &str, path: &Path, dev_link_targets: &HashSet<PathBuf>) -> bool {
    if dev_link_targets.contains(&file_io::canonical_or_original(path)) {
        return true;
    }
    if id.starts_with('.') {
        return true;
    }
    if path.extension().is_some_and(|ext| ext == "backup") {
        return true;
    }
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return true;
    };
    if metadata.file_type().is_symlink() {
        log::warn!("Skipping symlink in plugins dir: {}", id);
        return true;
    }
    !metadata.is_dir()
}

fn apply_dev_links(
    resolved: &mut HashMap<PluginId, ResolvedPlugin>,
    dev_links: &HashMap<String, PathBuf>,
) {
    for (raw_id, path) in dev_links {
        let id = PluginId::new(raw_id);
        if resolved.contains_key(&id) {
            log::info!("Dev-link overrides installed plugin: {}", raw_id);
        }
        resolved.insert(
            id.clone(),
            ResolvedPlugin {
                id,
                path: path.clone(),
                source: PluginSource::DevLinked,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn installed_plugin_resolved_from_dir() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("foo");
        fs::create_dir(&plugin_dir).unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id.as_str(), "foo");
        assert_eq!(result[0].source, PluginSource::Installed);
    }

    #[test]
    fn dev_link_added_when_not_installed() {
        let tmp = TempDir::new().unwrap();
        let dev_path = tmp.path().join("dev-src");
        fs::create_dir(&dev_path).unwrap();

        let dev_links = HashMap::from([("foo".to_string(), dev_path.clone())]);
        let result = resolve_all(tmp.path(), &dev_links);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id.as_str(), "foo");
        assert_eq!(result[0].path, dev_path);
        assert_eq!(result[0].source, PluginSource::DevLinked);
    }

    #[test]
    fn dev_link_overrides_installed() {
        let tmp = TempDir::new().unwrap();
        let installed = tmp.path().join("foo");
        fs::create_dir(&installed).unwrap();
        let dev_path = tmp.path().join("dev-foo");
        fs::create_dir(&dev_path).unwrap();

        let dev_links = HashMap::from([("foo".to_string(), dev_path.clone())]);
        let result = resolve_all(tmp.path(), &dev_links);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, dev_path);
        assert_eq!(result[0].source, PluginSource::DevLinked);
    }

    #[test]
    fn skips_backup_dirs() {
        let tmp = TempDir::new().unwrap();
        let backup = tmp.path().join("foo.backup");
        fs::create_dir(&backup).unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());
        assert!(result.is_empty());
    }

    #[test]
    fn skips_hidden_dirs() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".hidden")).unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());
        assert!(result.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("real");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, tmp.path().join("link")).unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id.as_str(), "real");
    }

    #[test]
    fn skips_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("not-a-dir"), "").unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());
        assert!(result.is_empty());
    }

    #[test]
    fn results_sorted_by_id() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("z-plugin")).unwrap();
        fs::create_dir(tmp.path().join("a-plugin")).unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());
        assert_eq!(result[0].id.as_str(), "a-plugin");
        assert_eq!(result[1].id.as_str(), "z-plugin");
    }

    #[test]
    fn mixed_installed_and_dev_linked() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("installed-only")).unwrap();
        let dev_path = tmp.path().join("dev-src");
        fs::create_dir(&dev_path).unwrap();

        let dev_links = HashMap::from([("dev-only".to_string(), dev_path)]);
        let result = resolve_all(tmp.path(), &dev_links);

        assert_eq!(result.len(), 2);
        let ids: Vec<&str> = result.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"installed-only"));
        assert!(ids.contains(&"dev-only"));
    }
}
