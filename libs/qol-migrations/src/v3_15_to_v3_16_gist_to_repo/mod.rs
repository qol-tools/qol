//! v3.15 -> v3.16 cloud migration: recover a pre-redesign profile from a
//! private GitHub gist into the local active profile dir.
//!
//! The legacy sync wrote a single `qol-tray-profile.json` blob into a private
//! gist described as `"QoL Tray Profile Sync"`. The new layout is a tree of
//! per-concern JSON files under `profile/<name>/`. This migration locates the
//! gist, transforms the blob with `transforms::gist_v1_to_layout`, writes the
//! tree to disk, stamps a sentinel marker, and ensures the repo-level
//! `.gitattributes` is present. The gist itself is left untouched on GitHub.

use crate::cloud::gist_store::{GistMetadata, GistStore, GitHubGistStore};
use crate::sentinel::ensure_marker_or_create;
use crate::transforms::gist_v1_to_layout::transform_gist_v1_to_layout;
use crate::{CloudMigration, MigrationContext, MigrationReport};
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const GIST_DESCRIPTION: &str = "QoL Tray Profile Sync";
const GIST_FILE_NAME: &str = "qol-tray-profile.json";
const MARKER_FILE_NAME: &str = ".qol-marker.json";
const SCHEMA_VERSION: u32 = 1;

pub struct V3_15ToV3_16GistToRepo {
    store: Arc<dyn GistStore>,
    active_profile_name: String,
}

impl V3_15ToV3_16GistToRepo {
    pub fn new(store: Arc<dyn GistStore>, active_profile_name: String) -> Self {
        Self {
            store,
            active_profile_name,
        }
    }

    /// Convenience factory for production code: uses [`GitHubGistStore`] and
    /// the well-known `default` profile name. Tests should call [`Self::new`]
    /// with a [`crate::cloud::gist_store::MemoryGistStore`] instead.
    pub fn default_for_production() -> Self {
        Self::new(Arc::new(GitHubGistStore::new()), "default".to_string())
    }

    fn profile_dir(&self, config_dir: &Path) -> PathBuf {
        config_dir.join("profile").join(&self.active_profile_name)
    }

    fn find_matching_gist(&self, gists: &[GistMetadata]) -> Result<Option<GistMetadata>> {
        let matches: Vec<&GistMetadata> = gists
            .iter()
            .filter(|m| {
                m.description == GIST_DESCRIPTION
                    && m.files.iter().any(|f| f == GIST_FILE_NAME)
            })
            .collect();

        match matches.len() {
            0 => Ok(None),
            1 => Ok(Some(matches[0].clone())),
            _ => Err(anyhow!(
                "ambiguous: multiple gists match {GIST_DESCRIPTION}"
            )),
        }
    }
}

#[async_trait::async_trait]
impl CloudMigration for V3_15ToV3_16GistToRepo {
    fn name(&self) -> &'static str {
        "v3.15-to-v3.16-gist-to-repo"
    }

    async fn applies(&self, ctx: &MigrationContext<'_>) -> Result<bool> {
        let Some(token) = ctx.github_token else {
            return Ok(false);
        };
        if self
            .profile_dir(ctx.config_dir)
            .join(MARKER_FILE_NAME)
            .exists()
        {
            return Ok(false);
        }
        let gists = self
            .store
            .list(token)
            .await
            .context("listing user gists for gist-to-repo applies check")?;
        Ok(self.find_matching_gist(&gists)?.is_some())
    }

    async fn migrate(
        &self,
        ctx: &MigrationContext<'_>,
        _archive_dir: &Path,
    ) -> Result<MigrationReport> {
        let token = ctx
            .github_token
            .ok_or_else(|| anyhow!("github token required for gist-to-repo migration"))?;

        let gists = self
            .store
            .list(token)
            .await
            .context("listing user gists for gist-to-repo migration")?;
        let gist = self
            .find_matching_gist(&gists)?
            .ok_or_else(|| anyhow!("no gist matches {GIST_DESCRIPTION}"))?;

        let raw = self
            .store
            .fetch_file(token, &gist.id, GIST_FILE_NAME)
            .await
            .with_context(|| format!("fetching {GIST_FILE_NAME} from gist {}", gist.id))?;

        let json: serde_json::Value =
            serde_json::from_str(&raw).context("parsing gist file as JSON")?;

        let target_os = match std::env::consts::OS {
            "linux" | "macos" | "windows" => std::env::consts::OS,
            _ => "linux",
        };

        crate::portability::validate_profile_name(&self.active_profile_name)
            .context("active profile name violates portability rules")?;

        let layout = transform_gist_v1_to_layout(&json, target_os)
            .context("transforming gist v1 to layout")?;

        let profile_dir = self.profile_dir(ctx.config_dir);
        for (rel_path, content) in &layout {
            let final_path = profile_dir.join(rel_path);
            atomic_write_file(&final_path, content).with_context(|| {
                format!("writing {} from gist", final_path.display())
            })?;
        }

        ensure_marker_or_create(
            &profile_dir.join(MARKER_FILE_NAME),
            None,
            &self.active_profile_name,
            SCHEMA_VERSION,
        )
        .context("writing sentinel marker after gist recovery")?;

        crate::portability::ensure_gitattributes(&ctx.config_dir.join("profile"))
            .context("ensuring .gitattributes for profile repo")?;

        Ok(MigrationReport {
            name: self.name().to_string(),
            archived: vec![],
        })
    }
}

fn atomic_write_file(final_path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = final_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating parent dir {}", parent.display()))?;
        }
    }
    let mut tmp = final_path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp: PathBuf = tmp.into();
    std::fs::write(&tmp, content)
        .with_context(|| format!("writing tmp {}", tmp.display()))?;
    std::fs::rename(&tmp, final_path).with_context(|| {
        format!(
            "renaming {} -> {}",
            tmp.display(),
            final_path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::gist_store::MemoryGistStore;
    use serde_json::json;
    use std::collections::HashMap;

    fn meta(id: &str, description: &str, files: &[&str]) -> GistMetadata {
        GistMetadata {
            id: id.to_string(),
            description: description.to_string(),
            files: files.iter().map(|s| s.to_string()).collect(),
            updated_at: "2026-05-23T00:00:00Z".to_string(),
            public: false,
        }
    }

    fn full_gist_blob() -> serde_json::Value {
        let mut hotkeys = Vec::new();
        for i in 0..13 {
            hotkeys.push(json!({
                "id": format!("hk-{i}"),
                "action": "toggle",
                "enabled": true,
                "key": format!("ctrl+{i}"),
                "plugin_id": "plugin-alt-tab",
            }));
        }
        json!({
            "version": 1,
            "hotkeys": hotkeys,
            "shortcuts": [
                {"id": "sc-1", "name": "Docs", "action": {"type": "open_url", "url": "https://example.test/1"}, "enabled": true, "export_to_launcher": true},
                {"id": "sc-2", "name": "Inbox", "action": {"type": "open_url", "url": "https://example.test/2"}, "enabled": true, "export_to_launcher": false},
                {"id": "sc-3", "name": "Calendar", "action": {"type": "open_url", "url": "https://example.test/3"}, "enabled": false, "export_to_launcher": true},
            ],
            "task_runner": {"actions": {"run-foo": {"cmd": "foo"}}},
            "plugin_configs": {
                "plugin-alt-tab": {"preview_size": 320},
                "plugin-launcher": {"max_results": 50},
                "plugin-lights": {"bridge": "zigbee2mqtt"},
                "plugin-window-actions": {"snap": true},
                "plugin-os-themes": {"dark": true},
                "plugin-screen-recorder": {"fps": 30},
            },
            "plugins": [
                {"id": "plugin-alt-tab", "repo_url": "https://example.test/alt-tab", "version": "1.2.3", "platforms": ["linux"]},
            ],
        })
    }

    fn store_with_matching_gist() -> Arc<MemoryGistStore> {
        let mut store = MemoryGistStore::new();
        let mut files = HashMap::new();
        files.insert(
            GIST_FILE_NAME.to_string(),
            full_gist_blob().to_string(),
        );
        store.add_gist(
            meta("matching-gist-id", GIST_DESCRIPTION, &[GIST_FILE_NAME]),
            files,
        );
        Arc::new(store)
    }

    fn migration_with(store: Arc<dyn GistStore>) -> V3_15ToV3_16GistToRepo {
        V3_15ToV3_16GistToRepo::new(store, "default".to_string())
    }

    #[tokio::test]
    async fn applies_returns_false_when_token_absent() {
        let dir = tempfile::tempdir().unwrap();
        let migration = migration_with(store_with_matching_gist());
        let ctx = MigrationContext {
            config_dir: dir.path(),
            github_token: None,
            http: None,
            host_version: "3.15.1",
        };
        assert!(!migration.applies(&ctx).await.unwrap());
    }

    #[tokio::test]
    async fn applies_returns_false_when_marker_already_written() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir
            .path()
            .join("profile/default")
            .join(MARKER_FILE_NAME);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, b"{}").unwrap();
        let migration = migration_with(store_with_matching_gist());
        let ctx = MigrationContext {
            config_dir: dir.path(),
            github_token: Some("tok"),
            http: None,
            host_version: "3.15.1",
        };
        assert!(!migration.applies(&ctx).await.unwrap());
    }

    #[tokio::test]
    async fn applies_returns_true_when_plugin_manager_wrote_lock_file_but_marker_absent() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir
            .path()
            .join("profile/default/core/plugins.lock.json");
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        std::fs::write(&lock, b"{}").unwrap();
        let migration = migration_with(store_with_matching_gist());
        let ctx = MigrationContext {
            config_dir: dir.path(),
            github_token: Some("tok"),
            http: None,
            host_version: "3.15.1",
        };
        assert!(migration.applies(&ctx).await.unwrap());
    }

    #[tokio::test]
    async fn applies_returns_false_when_no_gist_matches() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MemoryGistStore::new();
        store.add_gist(
            meta("other", "unrelated description", &["readme.md"]),
            HashMap::from([("readme.md".to_string(), "hi".to_string())]),
        );
        let migration = migration_with(Arc::new(store));
        let ctx = MigrationContext {
            config_dir: dir.path(),
            github_token: Some("tok"),
            http: None,
            host_version: "3.15.1",
        };
        assert!(!migration.applies(&ctx).await.unwrap());
    }

    #[tokio::test]
    async fn applies_returns_true_when_token_present_local_empty_one_matching_gist() {
        let dir = tempfile::tempdir().unwrap();
        let migration = migration_with(store_with_matching_gist());
        let ctx = MigrationContext {
            config_dir: dir.path(),
            github_token: Some("tok"),
            http: None,
            host_version: "3.15.1",
        };
        assert!(migration.applies(&ctx).await.unwrap());
    }

    #[tokio::test]
    async fn applies_errors_when_multiple_matching_gists() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MemoryGistStore::new();
        let mut files = HashMap::new();
        files.insert(GIST_FILE_NAME.to_string(), full_gist_blob().to_string());
        store.add_gist(
            meta("a", GIST_DESCRIPTION, &[GIST_FILE_NAME]),
            files.clone(),
        );
        store.add_gist(
            meta("b", GIST_DESCRIPTION, &[GIST_FILE_NAME]),
            files,
        );
        let migration = migration_with(Arc::new(store));
        let ctx = MigrationContext {
            config_dir: dir.path(),
            github_token: Some("tok"),
            http: None,
            host_version: "3.15.1",
        };
        let err = migration.applies(&ctx).await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("ambiguous") && msg.contains(GIST_DESCRIPTION),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    async fn migrate_writes_expected_file_tree() {
        let dir = tempfile::tempdir().unwrap();
        let archive_dir = dir.path().join("archive");
        std::fs::create_dir_all(&archive_dir).unwrap();
        let migration = migration_with(store_with_matching_gist());
        let ctx = MigrationContext {
            config_dir: dir.path(),
            github_token: Some("tok"),
            http: None,
            host_version: "3.15.1",
        };
        let report = migration.migrate(&ctx, &archive_dir).await.unwrap();
        assert_eq!(report.archived.len(), 0, "gist stays on GitHub");

        let profile_dir = dir.path().join("profile/default");
        let target_os = match std::env::consts::OS {
            "linux" | "macos" | "windows" => std::env::consts::OS,
            _ => "linux",
        };

        let cases: Vec<String> = vec![
            "manifest.json".to_string(),
            "core/plugins.lock.json".to_string(),
            format!("os/{target_os}/shortcuts.json"),
            format!("os/{target_os}/task-runner.json"),
            "core/plugin-configs/plugin-alt-tab.json".to_string(),
            "core/plugin-configs/plugin-launcher.json".to_string(),
            "core/plugin-configs/plugin-lights.json".to_string(),
            "core/plugin-configs/plugin-window-actions.json".to_string(),
            "core/plugin-configs/plugin-os-themes.json".to_string(),
            "core/plugin-configs/plugin-screen-recorder.json".to_string(),
        ];
        for path in &cases {
            assert!(
                profile_dir.join(path).is_file(),
                "expected file written: {path}"
            );
        }
        let hotkeys_path = profile_dir.join(format!("os/{target_os}/hotkeys.json"));
        assert!(
            hotkeys_path.is_file(),
            "hotkeys at os/{target_os}/hotkeys.json"
        );

        let lock: serde_json::Value =
            serde_json::from_slice(&std::fs::read(profile_dir.join("core/plugins.lock.json")).unwrap())
                .unwrap();
        assert_eq!(lock["plugins"][0]["id"], json!("plugin-alt-tab"));

        let hk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&hotkeys_path).unwrap()).unwrap();
        assert_eq!(hk["hotkeys"].as_array().unwrap().len(), 13);

        let sc: serde_json::Value = serde_json::from_slice(
            &std::fs::read(profile_dir.join(format!("os/{target_os}/shortcuts.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(sc["shortcuts"].as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn migrate_writes_sentinel_marker() {
        let dir = tempfile::tempdir().unwrap();
        let archive_dir = dir.path().join("archive");
        std::fs::create_dir_all(&archive_dir).unwrap();
        let migration = migration_with(store_with_matching_gist());
        let ctx = MigrationContext {
            config_dir: dir.path(),
            github_token: Some("tok"),
            http: None,
            host_version: "3.15.1",
        };
        migration.migrate(&ctx, &archive_dir).await.unwrap();

        let marker_path = dir.path().join("profile/default").join(MARKER_FILE_NAME);
        let marker = crate::sentinel::read_marker(&marker_path)
            .unwrap()
            .expect("marker written");
        assert_eq!(marker.profile_id, "default");
        assert_eq!(marker.schema_version, SCHEMA_VERSION);
        assert_eq!(marker.install_id.len(), 32, "freshly minted install_id");
    }

    #[tokio::test]
    async fn migrate_ensures_gitattributes_is_present() {
        let dir = tempfile::tempdir().unwrap();
        let archive_dir = dir.path().join("archive");
        std::fs::create_dir_all(&archive_dir).unwrap();
        let migration = migration_with(store_with_matching_gist());
        let ctx = MigrationContext {
            config_dir: dir.path(),
            github_token: Some("tok"),
            http: None,
            host_version: "3.15.1",
        };
        migration.migrate(&ctx, &archive_dir).await.unwrap();

        let attrs = dir.path().join("profile/.gitattributes");
        assert!(attrs.is_file(), ".gitattributes ensured at profile/");
        let content = std::fs::read_to_string(&attrs).unwrap();
        assert_eq!(content, crate::portability::GITATTRIBUTES_CONTENT);
    }
}
