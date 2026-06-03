use super::super::source::plugin_id_from_tree_path;
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

pub(super) fn collect_plugin_ids(tree: &TreeResponse) -> Vec<String> {
    if tree.truncated {
        log::warn!(
            "GitHub git/trees response is truncated; some plugins may be missing from discovery"
        );
    }
    let mut ids: Vec<String> = tree
        .tree
        .iter()
        .filter(|entry| entry.entry_type == "blob")
        .filter_map(|entry| plugin_id_from_tree_path(&entry.path).map(|id| id.to_string()))
        .collect();
    ids.sort();
    ids.dedup();
    ids
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
    fn collect_plugin_ids_filters_to_plugins_subdir_manifests() {
        let tree = TreeResponse {
            tree: vec![
                blob("plugins/plugin-alt-tab/plugin.toml"),
                blob("plugins/plugin-launcher/plugin.toml"),
                blob("plugins/plugin-launcher/Cargo.toml"),
                blob("plugins/plugin-launcher/src/main.rs"),
                blob("plugins/plugin-alt-tab/qol-config.toml"),
                blob("apps/qol-tray/plugin.toml"),
                blob("libs/qol-config/plugin.toml"),
                blob("Cargo.toml"),
                tree_node("plugins/plugin-alt-tab"),
                tree_node("plugins/plugin-alt-tab/src"),
            ],
            truncated: false,
        };
        let ids = collect_plugin_ids(&tree);
        assert_eq!(ids, vec!["plugin-alt-tab", "plugin-launcher"]);
    }

    #[test]
    fn collect_plugin_ids_ignores_tree_entries_with_matching_path() {
        let tree = TreeResponse {
            tree: vec![
                tree_node("plugins/plugin-alt-tab/plugin.toml"),
                blob("plugins/plugin-launcher/plugin.toml"),
            ],
            truncated: false,
        };
        let ids = collect_plugin_ids(&tree);
        assert_eq!(
            ids,
            vec!["plugin-launcher"],
            "directories named exactly plugin.toml must not be counted as plugins"
        );
    }

    #[test]
    fn collect_plugin_ids_dedupes() {
        let tree = TreeResponse {
            tree: vec![
                blob("plugins/plugin-alt-tab/plugin.toml"),
                blob("plugins/plugin-alt-tab/plugin.toml"),
            ],
            truncated: false,
        };
        let ids = collect_plugin_ids(&tree);
        assert_eq!(ids, vec!["plugin-alt-tab"]);
    }

    #[test]
    fn collect_plugin_ids_handles_empty_tree() {
        let tree = TreeResponse {
            tree: vec![],
            truncated: false,
        };
        assert!(collect_plugin_ids(&tree).is_empty());
    }

    #[test]
    fn tree_response_deserializes_minimal_github_payload() {
        let payload = r#"{
            "tree": [
                {"path": "plugins/plugin-alt-tab/plugin.toml", "type": "blob"},
                {"path": "plugins/plugin-launcher", "type": "tree"},
                {"path": "README.md", "type": "blob"}
            ],
            "truncated": false
        }"#;
        let tree: TreeResponse = serde_json::from_str(payload).expect("valid tree json");
        let ids = collect_plugin_ids(&tree);
        assert_eq!(ids, vec!["plugin-alt-tab"]);
    }
}
