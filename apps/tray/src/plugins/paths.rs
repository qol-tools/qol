use anyhow::Result;
use std::path::{Path, PathBuf};

const CONFIG_FILE_NAME: &str = "config.json";
const MANIFEST_FILE_NAME: &str = "plugin.toml";
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
    crate::paths::shared_config_dir()
        .ok()
        .and_then(|config_dir| {
            crate::plugins::resolver::resolve_plugin_path(&config_dir, plugin_id)
        })
        .unwrap_or_else(|| plugins_dir.join(plugin_id))
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

pub(crate) fn has_config(plugin_root: &Path) -> bool {
    config_contract_path(plugin_root).exists()
}

pub(crate) fn has_manifest(plugin_root: &Path) -> bool {
    plugin_root.join(MANIFEST_FILE_NAME).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::registry::{Entry, Registry, Slot, SlotSource};

    fn write_manifest(plugin_dir: &Path, id: &str) {
        std::fs::create_dir_all(plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join(MANIFEST_FILE_NAME),
            format!(
                "[plugin]\nid = \"{id}\"\nname = \"{id}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n",
            ),
        )
        .unwrap();
    }

    fn dev_linked_entry(id: &str, link: &Path, installed: &Path) -> Entry {
        Entry {
            id: id.to_string(),
            active: Slot {
                path: link.to_path_buf(),
                source: SlotSource::DevLink {
                    origin_path: link.to_path_buf(),
                },
            },
            fallback: Some(Slot {
                path: installed.to_path_buf(),
                source: SlotSource::ReleaseAsset,
            }),
        }
    }

    #[test]
    fn resolve_plugin_root_follows_the_registry_rather_than_the_plugins_dir() {
        let root = tempfile::TempDir::new().unwrap();
        let _paths = crate::paths::push_test_path_root(root.path());
        let installed = root.path().join("installed");
        let worktree = root.path().join("worktree");
        let deleted_worktree = root.path().join("deleted-worktree");
        write_manifest(&installed.join("plugin-dead"), "plugin-dead");
        write_manifest(&installed.join("plugin-live"), "plugin-live");
        write_manifest(&worktree.join("plugin-live"), "plugin-live");
        let config_dir = crate::paths::shared_config_dir().unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        crate::plugins::registry::save_registry(
            &config_dir,
            &Registry {
                version: 1,
                entries: vec![
                    dev_linked_entry(
                        "plugin-dead",
                        &deleted_worktree.join("plugin-dead"),
                        &installed.join("plugin-dead"),
                    ),
                    dev_linked_entry(
                        "plugin-live",
                        &worktree.join("plugin-live"),
                        &installed.join("plugin-live"),
                    ),
                ],
            },
        )
        .unwrap();
        let live_link_root = if cfg!(feature = "dev") {
            worktree.join("plugin-live")
        } else {
            installed.join("plugin-live")
        };

        let cases = [
            ("plugin-dead", installed.join("plugin-dead")),
            ("plugin-live", live_link_root),
            (
                "plugin-unlisted",
                crate::paths::plugins_dir().unwrap().join("plugin-unlisted"),
            ),
        ];
        for (plugin_id, expected) in cases {
            assert_eq!(
                resolve_plugin_root(plugin_id).unwrap(),
                expected,
                "plugin: {plugin_id}"
            );
        }
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
