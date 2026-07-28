use super::super::source::plugin_dir_from_tree_path;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct TreeResponse {
    #[serde(default)]
    pub(super) tree: Vec<TreeEntry>,
    #[serde(default)]
    pub(super) truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct TreeEntry {
    pub(super) path: String,
    #[serde(rename = "type", default)]
    pub(super) entry_type: String,
}

pub(super) fn collect_plugin_dirs(tree: &TreeResponse) -> Vec<String> {
    if tree.truncated {
        log::warn!(
            "GitHub git/trees response is truncated; some plugins may be missing from discovery"
        );
    }
    let mut dirs: Vec<String> = tree
        .tree
        .iter()
        .filter(|entry| entry.entry_type == "blob")
        .filter_map(|entry| plugin_dir_from_tree_path(&entry.path).map(str::to_string))
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(path: &str) -> TreeEntry {
        TreeEntry {
            path: path.to_string(),
            entry_type: "blob".to_string(),
        }
    }

    fn tree_node(path: &str) -> TreeEntry {
        TreeEntry {
            path: path.to_string(),
            entry_type: "tree".to_string(),
        }
    }

    #[test]
    fn collect_plugin_dirs_filters_to_plugins_subdir_manifests() {
        let tree = TreeResponse {
            tree: vec![
                blob("plugins/alt-tab/plugin.toml"),
                blob("plugins/launcher/plugin.toml"),
                blob("plugins/launcher/Cargo.toml"),
                blob("plugins/launcher/src/main.rs"),
                blob("plugins/alt-tab/qol-config.toml"),
                blob("apps/qol-tray/plugin.toml"),
                blob("libs/qol-config/plugin.toml"),
                blob("Cargo.toml"),
                tree_node("plugins/alt-tab"),
                tree_node("plugins/alt-tab/src"),
            ],
            truncated: false,
        };
        let dirs = collect_plugin_dirs(&tree);
        assert_eq!(dirs, vec!["alt-tab", "launcher"]);
    }

    #[test]
    fn collect_plugin_dirs_ignores_tree_entries_with_matching_path() {
        let tree = TreeResponse {
            tree: vec![
                tree_node("plugins/alt-tab/plugin.toml"),
                blob("plugins/launcher/plugin.toml"),
            ],
            truncated: false,
        };
        let dirs = collect_plugin_dirs(&tree);
        assert_eq!(
            dirs,
            vec!["launcher"],
            "directories named exactly plugin.toml must not be counted as plugins"
        );
    }

    #[test]
    fn collect_plugin_dirs_dedupes() {
        let tree = TreeResponse {
            tree: vec![
                blob("plugins/alt-tab/plugin.toml"),
                blob("plugins/alt-tab/plugin.toml"),
            ],
            truncated: false,
        };
        let dirs = collect_plugin_dirs(&tree);
        assert_eq!(dirs, vec!["alt-tab"]);
    }

    #[test]
    fn collect_plugin_dirs_handles_empty_tree() {
        let tree = TreeResponse {
            tree: vec![],
            truncated: false,
        };
        assert!(collect_plugin_dirs(&tree).is_empty());
    }

    #[test]
    fn tree_response_deserializes_minimal_github_payload() {
        let payload = r#"{
            "tree": [
                {"path": "plugins/alt-tab/plugin.toml", "type": "blob"},
                {"path": "plugins/launcher", "type": "tree"},
                {"path": "README.md", "type": "blob"}
            ],
            "truncated": false
        }"#;
        let tree: TreeResponse = serde_json::from_str(payload).expect("valid tree json");
        let dirs = collect_plugin_dirs(&tree);
        assert_eq!(dirs, vec!["alt-tab"]);
    }
}
