mod migration;

pub use migration::ensure_registry_initialized;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const REGISTRY_FILE_NAME: &str = "plugin-registry.json";
pub const CURRENT_REGISTRY_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registry {
    pub version: u32,
    pub entries: Vec<Entry>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            version: CURRENT_REGISTRY_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub active: Slot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Slot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Slot {
    pub path: PathBuf,
    pub source: SlotSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SlotSource {
    ReleaseAsset,
    DevLink {
        origin_path: PathBuf,
    },
    WorktreeLink {
        origin_path: PathBuf,
        branch: String,
    },
}

pub fn registry_path(config_dir: &Path) -> PathBuf {
    config_dir.join(REGISTRY_FILE_NAME)
}

pub fn lookup_active_path(config_dir: &Path, plugin_id: &str) -> Option<PathBuf> {
    let registry = load_registry(config_dir).ok()?;
    registry
        .entries
        .into_iter()
        .find(|e| e.id == plugin_id)
        .map(|e| e.active.path)
}

pub fn load_registry(config_dir: &Path) -> Result<Registry, String> {
    let path = registry_path(config_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
        Err(e) => return Err(format!("Failed to read {}: {}", path.display(), e)),
    };
    let registry: Registry = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
    if registry.version > CURRENT_REGISTRY_VERSION {
        return Err(format!(
            "Registry at {} is version {}; this binary supports up to version {}. \
             Downgrade detected — refusing to read a newer format.",
            path.display(),
            registry.version,
            CURRENT_REGISTRY_VERSION
        ));
    }
    Ok(registry)
}

pub fn save_registry(config_dir: &Path, registry: &Registry) -> Result<(), String> {
    let final_path = registry_path(config_dir);
    let tmp_path = config_dir.join(format!("{}.new", REGISTRY_FILE_NAME));
    let content = serde_json::to_string_pretty(registry)
        .map_err(|e| format!("Failed to serialize registry: {}", e))?;
    std::fs::write(&tmp_path, &content)
        .map_err(|e| format!("Failed to write {}: {}", tmp_path.display(), e))?;
    std::fs::rename(&tmp_path, &final_path)
        .map_err(|e| format!("Failed to finalize {}: {}", final_path.display(), e))?;
    let _ = fsync_dir(config_dir);
    Ok(())
}

fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_entry() -> Entry {
        Entry {
            id: "plugin-lights".to_string(),
            active: Slot {
                path: PathBuf::from("/home/user/dev/plugin-lights"),
                source: SlotSource::DevLink {
                    origin_path: PathBuf::from("/home/user/dev/plugin-lights"),
                },
            },
            fallback: Some(Slot {
                path: PathBuf::from("/home/user/.config/qol-tray/plugins/plugin-lights"),
                source: SlotSource::ReleaseAsset,
            }),
        }
    }

    #[test]
    fn default_registry_uses_current_version_and_no_entries() {
        let r = Registry::default();
        assert_eq!(r.version, CURRENT_REGISTRY_VERSION);
        assert!(r.entries.is_empty());
    }

    #[test]
    fn load_missing_file_returns_default() {
        let tmp = TempDir::new().unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry, Registry::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let registry = Registry {
            version: CURRENT_REGISTRY_VERSION,
            entries: vec![sample_entry()],
        };
        save_registry(tmp.path(), &registry).unwrap();
        let loaded = load_registry(tmp.path()).unwrap();
        assert_eq!(loaded, registry);
    }

    #[test]
    fn fallback_is_omitted_when_none() {
        let entry = Entry {
            id: "plugin-x".to_string(),
            active: Slot {
                path: PathBuf::from("/x"),
                source: SlotSource::ReleaseAsset,
            },
            fallback: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("fallback"));
    }

    #[test]
    fn slot_source_tags_are_kebab_case() {
        let release = serde_json::to_string(&SlotSource::ReleaseAsset).unwrap();
        assert!(release.contains("\"type\":\"release-asset\""));
        let dev = serde_json::to_string(&SlotSource::DevLink {
            origin_path: PathBuf::from("/p"),
        })
        .unwrap();
        assert!(dev.contains("\"type\":\"dev-link\""));
        let wt = serde_json::to_string(&SlotSource::WorktreeLink {
            origin_path: PathBuf::from("/p"),
            branch: "feat".to_string(),
        })
        .unwrap();
        assert!(wt.contains("\"type\":\"worktree-link\""));
    }

    #[test]
    fn malformed_json_returns_error() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(registry_path(tmp.path()), "{not json}").unwrap();
        let result = load_registry(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("parse"));
    }

    #[test]
    fn rejects_registry_with_future_version() {
        let tmp = TempDir::new().unwrap();
        let future = Registry {
            version: CURRENT_REGISTRY_VERSION + 1,
            entries: vec![],
        };
        let content = serde_json::to_string_pretty(&future).unwrap();
        std::fs::write(registry_path(tmp.path()), content).unwrap();
        let result = load_registry(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Downgrade"));
    }
}
