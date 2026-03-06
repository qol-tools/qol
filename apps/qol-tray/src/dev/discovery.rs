mod manifest;
mod output;
mod search;
mod source;

pub use output::{discover_plugins, DiscoveredPlugin};

#[cfg(test)]
mod tests {
    use super::discover_plugins;
    use super::search::find_plugin_dirs;
    use crate::dev::DevConfig;
    use std::fs;
    use std::path::Path;
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
        let deep = tmp
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("d")
            .join("e")
            .join("f");
        fs::create_dir_all(&deep).unwrap();
        create_plugin_toml(&deep);

        let found = find_plugin_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 0);
    }

    #[test]
    fn finds_plugin_at_depth_3() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp
            .path()
            .join("private")
            .join("pointZ")
            .join("PointZerver");
        fs::create_dir_all(&plugin_dir).unwrap();
        create_plugin_toml(&plugin_dir);

        let found = find_plugin_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 1, "Should find plugin at depth 3");
        assert_eq!(found[0], plugin_dir);
    }

    #[test]
    fn finds_plugin_at_depth_5() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("d")
            .join("plugin");
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
            search_paths: vec![tmp.path().to_path_buf(), tmp.path().join("sub")],
        };

        let discovered = discover_plugins(&config, tmp.path());
        assert_eq!(
            discovered.len(),
            1,
            "Should deduplicate plugin found from multiple search roots"
        );
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
        assert_eq!(
            discovered.len(),
            1,
            "Should find it even if TOML is minimal"
        );
        assert_eq!(discovered[0].name, "Minimal");
    }
}
