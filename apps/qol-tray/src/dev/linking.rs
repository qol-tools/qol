mod listing;
mod store;

use serde::{Deserialize, Serialize};

pub use listing::list_linked_plugins;
pub(crate) use store::set_active_worktree_branch;
pub use store::{create_link, get_active_worktree_branch, remove_link};

pub fn active_dev_links(
    config_dir: &std::path::Path,
) -> std::collections::HashMap<String, std::path::PathBuf> {
    let base = crate::plugins::registry::dev_linked_paths(config_dir);
    crate::dev::resolve_worktree_paths(&base, get_active_worktree_branch(config_dir).as_deref())
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkedPlugin {
    pub id: String,
    pub name: String,
    pub source: String,
    pub has_cargo: bool,
    pub supports_platform: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn write_plugin_toml(dir: &Path, name: &str) {
        let id = dir.file_name().unwrap().to_str().unwrap();
        fs::write(
            dir.join("plugin.toml"),
            format!(
                "[plugin]\nid = \"{id}\"\nname = \"{name}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n",
            ),
        )
        .unwrap();
    }

    #[test]
    fn dev_linked_paths_empty_when_no_registry() {
        let tmp = TempDir::new().unwrap();
        assert!(crate::plugins::registry::dev_linked_paths(tmp.path()).is_empty());
    }

    #[test]
    fn create_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("my-plugin");
        fs::create_dir(&source).unwrap();
        write_plugin_toml(&source, "My Plugin");

        let id = create_link(&source, tmp.path()).unwrap();
        assert_eq!(id, "my-plugin");

        let links = crate::plugins::registry::dev_linked_paths(tmp.path());
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

        assert!(crate::plugins::registry::dev_linked_paths(tmp.path()).is_empty());
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
