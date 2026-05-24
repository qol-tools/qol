use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::{FileMigration, MigrationReport};

pub struct V3_17ToV3_18PluginConfigsByOs {
    target_os: &'static str,
}

impl V3_17ToV3_18PluginConfigsByOs {
    pub fn new_for_os(target_os: &'static str) -> Self {
        Self { target_os }
    }

    pub fn default_for_production() -> Self {
        Self::new_for_os(current_os_subdir())
    }
}

fn current_os_subdir() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

#[derive(Debug, Deserialize)]
struct LockEntry {
    id: String,
    #[serde(default)]
    platforms: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct LockFile {
    #[serde(default)]
    plugins: Vec<LockEntry>,
}

#[derive(Debug, Deserialize, Default)]
struct PluginToml {
    #[serde(default)]
    plugin: PluginToml_Plugin,
    #[serde(default)]
    config: PluginToml_Config,
}

#[derive(Debug, Deserialize, Default)]
#[allow(non_camel_case_types)]
struct PluginToml_Plugin {
    #[serde(default)]
    platforms: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
#[allow(non_camel_case_types)]
struct PluginToml_Config {
    #[serde(default)]
    default_scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TargetScope {
    StayInCore,
    MoveToOs(String),
    MoveToDevice,
}

fn resolve_target_scope(
    lock: Option<&LockEntry>,
    manifest: Option<&PluginToml>,
    current_os: &str,
) -> TargetScope {
    let lock_single = lock.and_then(|e| single_platform(e.platforms.as_deref()));
    let manifest_single =
        manifest.and_then(|m| single_platform(m.plugin.platforms.as_deref()));

    if let Some(scope) = manifest.and_then(|m| m.config.default_scope.as_deref()) {
        return match scope {
            "device" => TargetScope::MoveToDevice,
            "os" => {
                let bucket = lock_single
                    .or(manifest_single)
                    .unwrap_or_else(|| current_os.to_string());
                TargetScope::MoveToOs(bucket)
            }
            "core" | "any" => TargetScope::StayInCore,
            other => {
                log::warn!(
                    "[v3.17-to-v3.18] unknown config default_scope {other:?}; leaving in core"
                );
                TargetScope::StayInCore
            }
        };
    }

    if let Some(bucket) = lock_single.or(manifest_single) {
        return TargetScope::MoveToOs(bucket);
    }
    TargetScope::StayInCore
}

fn single_platform(platforms: Option<&[String]>) -> Option<String> {
    match platforms {
        Some([only]) => Some(only.clone()),
        _ => None,
    }
}

fn is_safe_path_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

fn list_profile_dirs(profile_root: &Path) -> Result<Vec<PathBuf>> {
    if !profile_root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(profile_root)
        .with_context(|| format!("reading {}", profile_root.display()))?
    {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("manifest.json").is_file() {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

fn read_lock(profile_dir: &Path) -> Option<LockFile> {
    let path = profile_dir.join("core").join("plugins.lock.json");
    if !path.is_file() {
        return None;
    }
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn read_plugin_manifest(plugins_dir: &Path, plugin_id: &str) -> Option<PluginToml> {
    if !is_safe_path_component(plugin_id) {
        return None;
    }
    let path = plugins_dir.join(plugin_id).join("plugin.toml");
    if !path.is_file() {
        return None;
    }
    let raw = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&raw).ok()
}

fn legacy_sidecar_path(src: &Path) -> PathBuf {
    let mut name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(".legacy");
    src.with_file_name(name)
}

impl FileMigration for V3_17ToV3_18PluginConfigsByOs {
    fn name(&self) -> &'static str {
        "v3.17-to-v3.18-plugin-configs-by-os"
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        let profile_root = config_dir.join("profile");
        for profile_dir in list_profile_dirs(&profile_root)? {
            let core_configs = profile_dir.join("core").join("plugin-configs");
            if !core_configs.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(&core_configs).with_context(|| {
                format!("reading {}", core_configs.display())
            })? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn migrate(&self, config_dir: &Path, _archive_dir: &Path) -> Result<MigrationReport> {
        let profile_root = config_dir.join("profile");
        let plugins_dir = config_dir.join("plugins");
        let mut touched = Vec::new();

        for profile_dir in list_profile_dirs(&profile_root)? {
            let core_configs = profile_dir.join("core").join("plugin-configs");
            if !core_configs.is_dir() {
                continue;
            }
            let lock = read_lock(&profile_dir);
            let entries: Vec<_> = std::fs::read_dir(&core_configs)
                .with_context(|| format!("reading {}", core_configs.display()))?
                .filter_map(|e| e.ok())
                .collect();

            for entry in entries {
                let src = entry.path();
                if !src.is_file() {
                    continue;
                }
                if src.extension().is_none_or(|e| e != "json") {
                    continue;
                }
                let Some(plugin_id) = src.file_stem().and_then(|s| s.to_str()) else {
                    log::warn!(
                        "[v3.17-to-v3.18] skipping unreadable file name at {}",
                        src.display()
                    );
                    continue;
                };
                if !is_safe_path_component(plugin_id) {
                    log::warn!(
                        "[v3.17-to-v3.18] skipping unsafe plugin id {plugin_id:?} at {}",
                        src.display()
                    );
                    continue;
                }
                if plugin_id.ends_with(".legacy") {
                    continue;
                }

                let lock_entry = lock
                    .as_ref()
                    .and_then(|l| l.plugins.iter().find(|e| e.id == plugin_id));
                let manifest = read_plugin_manifest(&plugins_dir, plugin_id);
                let scope = resolve_target_scope(lock_entry, manifest.as_ref(), self.target_os);

                let dst = match &scope {
                    TargetScope::StayInCore => continue,
                    TargetScope::MoveToOs(bucket) => {
                        if !is_safe_path_component(bucket) {
                            log::warn!(
                                "[v3.17-to-v3.18] skipping {plugin_id}: unsafe os bucket {bucket:?}"
                            );
                            continue;
                        }
                        profile_dir
                            .join("os")
                            .join(bucket)
                            .join("plugin-configs")
                            .join(format!("{plugin_id}.json"))
                    }
                    TargetScope::MoveToDevice => profile_dir
                        .join("device")
                        .join("plugin-configs")
                        .join(format!("{plugin_id}.json")),
                };

                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }

                if dst.exists() {
                    if !dst.is_file() {
                        return Err(anyhow!(
                            "destination path exists but is not a file: {}",
                            dst.display()
                        ));
                    }
                    let src_bytes = std::fs::read(&src)
                        .with_context(|| format!("reading {}", src.display()))?;
                    let dst_bytes = std::fs::read(&dst)
                        .with_context(|| format!("reading {}", dst.display()))?;
                    if src_bytes == dst_bytes {
                        std::fs::remove_file(&src).with_context(|| {
                            format!("removing redundant src {}", src.display())
                        })?;
                        log::info!(
                            "[v3.17-to-v3.18] {plugin_id}: src and dst at {} are identical; removed src",
                            dst.display()
                        );
                    } else {
                        let bak = legacy_sidecar_path(&src);
                        if bak.exists() {
                            std::fs::remove_file(&bak).with_context(|| {
                                format!("clearing prior sidecar {}", bak.display())
                            })?;
                        }
                        std::fs::rename(&src, &bak).with_context(|| {
                            format!(
                                "destination {} differs; archiving legacy {} to {}",
                                dst.display(),
                                src.display(),
                                bak.display()
                            )
                        })?;
                        log::warn!(
                            "[v3.17-to-v3.18] {plugin_id}: destination {} differs from src; preserved legacy at {}",
                            dst.display(),
                            bak.display()
                        );
                    }
                } else {
                    std::fs::rename(&src, &dst).with_context(|| {
                        format!("renaming {} to {}", src.display(), dst.display())
                    })?;
                    touched.push(dst);
                }
            }
        }

        Ok(MigrationReport {
            name: self.name().to_string(),
            archived: touched,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OS_MAC: &str = "macos";
    const OS_LINUX: &str = "linux";

    fn migration(target_os: &'static str) -> V3_17ToV3_18PluginConfigsByOs {
        V3_17ToV3_18PluginConfigsByOs::new_for_os(target_os)
    }

    fn empty_archive(dir: &Path) -> PathBuf {
        let p = dir.join("archive").join("v3.17-to-v3.18-test");
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn read(path: &Path) -> Vec<u8> {
        std::fs::read(path).unwrap()
    }

    fn setup_profile(config_dir: &Path, name: &str) -> PathBuf {
        let root = config_dir.join("profile").join(name);
        write(&root.join("manifest.json"), b"{\"version\":1}");
        root
    }

    fn write_lock(profile_root: &Path, entries: &[(&str, &[&str])]) {
        let json = serde_json::json!({
            "version": 1,
            "plugins": entries.iter().map(|(id, p)| serde_json::json!({
                "id": id,
                "repo_url": "x",
                "version": "1.0.0",
                "platforms": p,
            })).collect::<Vec<_>>(),
        });
        write(
            &profile_root.join("core").join("plugins.lock.json"),
            json.to_string().as_bytes(),
        );
    }

    fn write_plugin_manifest(config_dir: &Path, plugin_id: &str, manifest_toml: &str) {
        write(
            &config_dir
                .join("plugins")
                .join(plugin_id)
                .join("plugin.toml"),
            manifest_toml.as_bytes(),
        );
    }

    #[test]
    fn applies_is_false_when_no_profile_dir_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!migration(OS_MAC).applies(dir.path()).unwrap());
    }

    #[test]
    fn applies_is_false_when_profile_has_no_core_plugin_configs() {
        let dir = tempfile::tempdir().unwrap();
        setup_profile(dir.path(), "default");
        assert!(!migration(OS_MAC).applies(dir.path()).unwrap());
    }

    #[test]
    fn applies_is_true_when_at_least_one_core_plugin_config_exists() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("core").join("plugin-configs").join("p.json"), b"{}");
        assert!(migration(OS_MAC).applies(dir.path()).unwrap());
    }

    #[test]
    fn lock_single_platform_routes_core_config_to_that_os_bucket() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root.join("core").join("plugin-configs").join("plugin-keyremap.json");
        write(&src, br#"{"enabled":true}"#);
        write_lock(&root, &[("plugin-keyremap", &["macos"])]);

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(!src.exists(), "src removed once moved");
        let dst = root
            .join("os")
            .join("macos")
            .join("plugin-configs")
            .join("plugin-keyremap.json");
        assert!(dst.is_file());
        assert_eq!(read(&dst), br#"{"enabled":true}"#);
    }

    #[test]
    fn lock_wins_over_manifest_when_both_declare_a_single_but_different_platform() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root
            .join("core")
            .join("plugin-configs")
            .join("plugin-divergent.json");
        write(&src, b"\"x\"");
        write_lock(&root, &[("plugin-divergent", &["macos"])]);
        write_plugin_manifest(
            dir.path(),
            "plugin-divergent",
            r#"
[plugin]
name = "p"
description = ""
version = "1.0.0"
platforms = ["linux"]

[menu]
label = "p"
items = []
"#,
        );

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(root
            .join("os/macos/plugin-configs/plugin-divergent.json")
            .is_file());
        assert!(!root
            .join("os/linux/plugin-configs/plugin-divergent.json")
            .exists());
    }

    #[test]
    fn manifest_single_platform_routes_when_lock_is_absent_for_that_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root
            .join("core")
            .join("plugin-configs")
            .join("plugin-only-win.json");
        write(&src, b"{}");
        write_plugin_manifest(
            dir.path(),
            "plugin-only-win",
            r#"
[plugin]
name = "p"
description = ""
version = "1.0.0"
platforms = ["windows"]

[menu]
label = "p"
items = []
"#,
        );

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(root
            .join("os/windows/plugin-configs/plugin-only-win.json")
            .is_file());
    }

    #[test]
    fn missing_plugin_with_no_lock_and_no_manifest_stays_in_core() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root
            .join("core")
            .join("plugin-configs")
            .join("plugin-unknown.json");
        write(&src, b"{\"preserve\":true}");

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(
            src.is_file(),
            "unclassifiable plugin must stay in core, not be moved to a guessed bucket"
        );
        assert_eq!(read(&src), b"{\"preserve\":true}");
        assert!(!root.join("os").exists() || root.join("os").read_dir().unwrap().next().is_none());
    }

    #[test]
    fn multi_platform_lock_without_default_scope_keeps_config_in_core() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root
            .join("core")
            .join("plugin-configs")
            .join("plugin-cross.json");
        write(&src, b"{}");
        write_lock(&root, &[("plugin-cross", &["linux", "macos"])]);

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(
            src.is_file(),
            "multi-platform plugin must stay in core unless the author opts otherwise"
        );
    }

    #[test]
    fn manifest_default_scope_device_routes_to_device_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root
            .join("core")
            .join("plugin-configs")
            .join("plugin-secrets.json");
        write(&src, b"{\"broker\":\"x\"}");
        write_plugin_manifest(
            dir.path(),
            "plugin-secrets",
            r#"
[plugin]
name = "p"
description = ""
version = "1.0.0"
platforms = ["linux", "macos"]

[menu]
label = "p"
items = []

[config]
default_scope = "device"
"#,
        );

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(root
            .join("device/plugin-configs/plugin-secrets.json")
            .is_file());
        assert!(!src.exists());
    }

    #[test]
    fn same_content_collision_drops_src_without_creating_a_legacy_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root
            .join("core")
            .join("plugin-configs")
            .join("plugin-keyremap.json");
        let dst = root
            .join("os")
            .join("macos")
            .join("plugin-configs")
            .join("plugin-keyremap.json");
        write(&src, b"\"same\"");
        write(&dst, b"\"same\"");
        write_lock(&root, &[("plugin-keyremap", &["macos"])]);

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(!src.exists(), "redundant src removed when content matches dst");
        let legacy = root
            .join("core")
            .join("plugin-configs")
            .join("plugin-keyremap.json.legacy");
        assert!(
            !legacy.exists(),
            "no sidecar needed when content already matches"
        );
        assert_eq!(read(&dst), b"\"same\"");
    }

    #[test]
    fn different_content_collision_archives_src_to_legacy_sidecar_and_leaves_dst_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root
            .join("core")
            .join("plugin-configs")
            .join("plugin-keyremap.json");
        let dst = root
            .join("os")
            .join("macos")
            .join("plugin-configs")
            .join("plugin-keyremap.json");
        write(&src, b"\"src-data\"");
        write(&dst, b"\"dst-data\"");
        write_lock(&root, &[("plugin-keyremap", &["macos"])]);

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(!src.exists(), "src moved out of core");
        let legacy = root
            .join("core")
            .join("plugin-configs")
            .join("plugin-keyremap.json.legacy");
        assert!(legacy.is_file(), "src preserved at .legacy");
        assert_eq!(
            read(&legacy),
            b"\"src-data\"",
            "legacy holds the original src bytes verbatim"
        );
        assert_eq!(
            read(&dst),
            b"\"dst-data\"",
            "existing dst wins; never clobbered"
        );
    }

    #[test]
    fn running_the_migration_twice_in_a_row_is_a_no_op_on_the_second_pass() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root
            .join("core")
            .join("plugin-configs")
            .join("plugin-keyremap.json");
        write(&src, b"\"data\"");
        write_lock(&root, &[("plugin-keyremap", &["macos"])]);

        let m = migration(OS_LINUX);
        let first = m
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();
        assert_eq!(first.archived.len(), 1);

        let second = m
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();
        assert!(
            second.archived.is_empty(),
            "second pass moves nothing - core/plugin-configs is empty"
        );
        let dst = root
            .join("os/macos/plugin-configs/plugin-keyremap.json");
        assert_eq!(read(&dst), b"\"data\"");
    }

    #[test]
    fn unsafe_plugin_ids_are_skipped_without_constructing_any_path_outside_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let configs = root.join("core").join("plugin-configs");
        std::fs::create_dir_all(&configs).unwrap();

        let canary = dir.path().join("canary-outside-profile.json");
        write(&canary, b"untouched");

        let unsafe_names = ["..json", "....json"];
        for name in unsafe_names {
            write(&configs.join(name), b"{}");
        }
        let legit = configs.join("legit-plugin.json");
        write(&legit, b"{}");
        write_lock(&root, &[("legit-plugin", &["macos"])]);

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(
            canary.is_file(),
            "out-of-profile canary must never be touched no matter how dodgy the file names are"
        );
        assert_eq!(read(&canary), b"untouched");

        for name in unsafe_names {
            assert!(
                configs.join(name).is_file(),
                "unsafe name {name:?} must be skipped, not moved or deleted"
            );
        }

        assert!(
            root.join("os/macos/plugin-configs/legit-plugin.json")
                .is_file(),
            "legit entries still process even when unsafe ones share the directory"
        );
    }

    #[test]
    fn legacy_sidecar_files_are_ignored_on_subsequent_runs() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let configs = root.join("core").join("plugin-configs");
        write(&configs.join("p.json.legacy"), b"\"legacy\"");

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(
            configs.join("p.json.legacy").is_file(),
            ".legacy sidecars must not be picked up as migration sources"
        );
    }

    #[test]
    fn multi_profile_setup_routes_each_profile_independently() {
        let dir = tempfile::tempdir().unwrap();
        let p_default = setup_profile(dir.path(), "default");
        let p_work = setup_profile(dir.path(), "work");
        write(
            &p_default.join("core/plugin-configs/plugin-x.json"),
            b"\"default\"",
        );
        write(
            &p_work.join("core/plugin-configs/plugin-x.json"),
            b"\"work\"",
        );
        write_lock(&p_default, &[("plugin-x", &["macos"])]);
        write_lock(&p_work, &[("plugin-x", &["linux"])]);

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert_eq!(
            read(&p_default.join("os/macos/plugin-configs/plugin-x.json")),
            b"\"default\""
        );
        assert_eq!(
            read(&p_work.join("os/linux/plugin-configs/plugin-x.json")),
            b"\"work\""
        );
    }

    #[test]
    fn report_lists_only_freshly_moved_files_not_collision_archives() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let fresh = root.join("core/plugin-configs/plugin-fresh.json");
        let collide_src = root.join("core/plugin-configs/plugin-collide.json");
        let collide_dst = root.join("os/macos/plugin-configs/plugin-collide.json");
        write(&fresh, b"\"f\"");
        write(&collide_src, b"\"different-src\"");
        write(&collide_dst, b"\"different-dst\"");
        write_lock(
            &root,
            &[
                ("plugin-fresh", &["macos"]),
                ("plugin-collide", &["macos"]),
            ],
        );

        let report = migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert_eq!(report.archived.len(), 1);
        let names: Vec<String> = report
            .archived
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["plugin-fresh.json"]);
    }
}
