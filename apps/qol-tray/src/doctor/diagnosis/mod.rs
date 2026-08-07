mod platform;

use super::de_bindings::filter_unshadow;
use super::install_id::write_install_id_file;
use crate::hotkeys::takeover::{self, BindingMutation, BindingReach};
use crate::plugins::daemon_tracker::ManagedProcess;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FixAction {
    SetActiveInstallId(String),
    WriteInstallMarker {
        marker_path: PathBuf,
        install_id: String,
    },
    WriteAutostartEntry {
        binary_path: PathBuf,
    },
    EnsurePluginsDir {
        path: PathBuf,
    },
    KillPluginProcessLeaks {
        processes: Vec<ManagedProcess>,
    },
    DrainOrphanPluginConfigs,
    InstallShellHook,
    UnshadowDeBinding {
        dir: String,
        key: String,
        qol_combo: String,
        orphaned: bool,
    },
    DisableSymbolicHotkey {
        hotkey_id: u32,
        qol_combo: String,
    },
    ClearWindowsAppKey {
        app_key: String,
        qol_combo: String,
    },
    HoldNvidiaDriverPackages,
    UnholdNvidiaDriverPackages,
    ApplyHeldNvidiaDriverUpdate {
        packages: Vec<String>,
    },
    #[cfg(feature = "dev")]
    RelocateDevLink {
        plugin_id: String,
        to: PathBuf,
    },
    #[cfg(feature = "dev")]
    PruneOrphanFingerprints {
        ids: Vec<String>,
    },
    #[cfg(feature = "dev")]
    PruneReservedPlugins {
        ids: Vec<String>,
    },
    #[cfg(feature = "dev")]
    RemoveDevLinkEntries {
        ids: Vec<String>,
    },
    #[cfg(feature = "dev")]
    FormatRustSources {
        workspace: PathBuf,
    },
    #[cfg(feature = "dev")]
    FixClippyLints {
        workspace: PathBuf,
    },
    #[cfg(feature = "dev")]
    PruneCargoIncrementalCache {
        path: PathBuf,
    },
    #[cfg(feature = "dev")]
    PruneCargoTargetDir {
        target: PathBuf,
    },
    #[cfg(feature = "dev")]
    HealDevLinkedPlugins {
        rebuild_ids: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum FixApplicability {
    #[default]
    SafeAutomatic,
    ReversibleHostMutation,
    ManualOnly,
}

impl FixAction {
    pub(super) fn applicability(&self) -> FixApplicability {
        match self {
            FixAction::SetActiveInstallId(_)
            | FixAction::WriteInstallMarker { .. }
            | FixAction::WriteAutostartEntry { .. }
            | FixAction::EnsurePluginsDir { .. }
            | FixAction::KillPluginProcessLeaks { .. }
            | FixAction::DrainOrphanPluginConfigs
            | FixAction::InstallShellHook => FixApplicability::SafeAutomatic,
            FixAction::UnshadowDeBinding { .. }
            | FixAction::DisableSymbolicHotkey { .. }
            | FixAction::ClearWindowsAppKey { .. } => FixApplicability::ReversibleHostMutation,
            FixAction::HoldNvidiaDriverPackages
            | FixAction::UnholdNvidiaDriverPackages
            | FixAction::ApplyHeldNvidiaDriverUpdate { .. } => FixApplicability::ManualOnly,
            #[cfg(feature = "dev")]
            FixAction::RelocateDevLink { .. }
            | FixAction::PruneOrphanFingerprints { .. }
            | FixAction::PruneReservedPlugins { .. }
            | FixAction::RemoveDevLinkEntries { .. }
            | FixAction::FormatRustSources { .. }
            | FixAction::FixClippyLints { .. }
            | FixAction::PruneCargoIncrementalCache { .. }
            | FixAction::PruneCargoTargetDir { .. }
            | FixAction::HealDevLinkedPlugins { .. } => FixApplicability::SafeAutomatic,
        }
    }

    pub(super) fn requires_workspace_fix_window(&self) -> bool {
        #[cfg(feature = "dev")]
        {
            if matches!(
                self,
                FixAction::FormatRustSources { .. }
                    | FixAction::FixClippyLints { .. }
                    | FixAction::PruneCargoIncrementalCache { .. }
                    | FixAction::PruneCargoTargetDir { .. }
                    | FixAction::HealDevLinkedPlugins { .. }
            ) {
                return true;
            }
        }
        false
    }
}

pub(super) fn apply_fix(action: &FixAction) -> Result<()> {
    match action {
        FixAction::SetActiveInstallId(install_id) => {
            crate::paths::set_active_install_id(install_id)
        }
        FixAction::WriteInstallMarker {
            marker_path,
            install_id,
        } => write_install_id_file(marker_path, install_id),
        FixAction::WriteAutostartEntry { binary_path } => {
            crate::installer::autostart::write_target(binary_path)
        }
        FixAction::EnsurePluginsDir { path } => {
            fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))
        }
        FixAction::KillPluginProcessLeaks { processes } => {
            crate::plugins::daemon_tracker::kill_managed_processes(processes);
            Ok(())
        }
        FixAction::DrainOrphanPluginConfigs => {
            let _drained = crate::config_drain::drain_orphan_runtime_configs();
            Ok(())
        }
        FixAction::InstallShellHook => crate::installer::install_shell_hook(),
        FixAction::UnshadowDeBinding {
            dir,
            key,
            qol_combo,
            orphaned,
        } => apply_unshadow(
            &UnshadowRequest {
                dir,
                key,
                qol_combo,
                orphaned: *orphaned,
            },
            &mut DconfTakeover,
        ),
        FixAction::DisableSymbolicHotkey {
            hotkey_id,
            qol_combo,
        } => apply_disable_symbolic_hotkey(*hotkey_id, qol_combo, &mut platform::Platform),
        FixAction::ClearWindowsAppKey { app_key, qol_combo } => {
            apply_clear_windows_app_key(app_key, qol_combo, &mut platform::Platform)
        }
        FixAction::HoldNvidiaDriverPackages => super::checks::hold_nvidia_driver_packages(),
        FixAction::UnholdNvidiaDriverPackages => super::checks::unhold_nvidia_driver_packages(),
        FixAction::ApplyHeldNvidiaDriverUpdate { packages } => {
            super::checks::apply_held_nvidia_driver_update(packages)
        }
        #[cfg(feature = "dev")]
        FixAction::RelocateDevLink { plugin_id, to } => {
            let config_dir = crate::paths::shared_config_dir()?;
            super::checks::relocate_dev_link(&config_dir, plugin_id, to)
                .map_err(|e| anyhow!("failed to relocate dev-link for {plugin_id}: {e}"))
        }
        #[cfg(feature = "dev")]
        FixAction::PruneOrphanFingerprints { ids } => prune_orphan_fingerprints(ids),
        #[cfg(feature = "dev")]
        FixAction::PruneReservedPlugins { ids } => prune_reserved_plugins(ids),
        #[cfg(feature = "dev")]
        FixAction::RemoveDevLinkEntries { ids } => remove_dev_link_entries(ids),
        #[cfg(feature = "dev")]
        FixAction::FormatRustSources { workspace } => format_rust_sources(workspace),
        #[cfg(feature = "dev")]
        FixAction::FixClippyLints { workspace } => fix_clippy_lints(workspace),
        #[cfg(feature = "dev")]
        FixAction::PruneCargoIncrementalCache { path } => prune_cargo_incremental_cache(path),
        #[cfg(feature = "dev")]
        FixAction::PruneCargoTargetDir { target } => {
            qol_dev_build::target_cache::prune_cargo_target_dir(target)
                .map_err(|error| anyhow!(error))
        }
        #[cfg(feature = "dev")]
        FixAction::HealDevLinkedPlugins { rebuild_ids } => heal_dev_linked_plugins(rebuild_ids),
    }
}

#[cfg(feature = "dev")]
fn prune_cargo_incremental_cache(path: &std::path::Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

#[cfg(feature = "dev")]
fn heal_dev_linked_plugins(rebuild_ids: &[String]) -> Result<()> {
    if !rebuild_ids.is_empty() {
        rebuild_dev_linked_plugins(rebuild_ids)?;
    }
    reload_stale_dev_daemons();
    Ok(())
}

#[cfg(feature = "dev")]
fn reload_stale_dev_daemons() {
    let Ok(config_dir) = crate::paths::shared_config_dir() else {
        return;
    };
    for (plugin_id, _) in super::checks::stale_running_daemons(&config_dir) {
        if let Err(error) = request_daemon_reload(&plugin_id) {
            log::warn!("doctor heal: could not reload stale daemon {plugin_id}: {error}");
        }
    }
}

#[cfg(feature = "dev")]
fn request_daemon_reload(plugin_id: &str) -> Result<()> {
    let path = qol_conventions::dev_routes::api_path(&qol_conventions::dev_routes::reload_plugin(
        plugin_id,
    ));
    let (status, _) = crate::local_http::post_to_daemon(&path, "")?;
    if status == 200 || status == 202 {
        Ok(())
    } else {
        Err(anyhow!("reload endpoint returned: {status}"))
    }
}

#[cfg(feature = "dev")]
struct NoopEventSink;

#[cfg(feature = "dev")]
impl crate::dev::adapters::CoreEventSink for NoopEventSink {
    fn publish(&self, _event: crate::dev::core::CoreEvent) {}
}

#[cfg(feature = "dev")]
fn rebuild_dev_linked_plugins(ids: &[String]) -> Result<()> {
    let config_dir = crate::paths::shared_config_dir()?;

    let dev_links: std::collections::HashMap<String, PathBuf> =
        crate::plugins::registry::dev_linked_paths(&config_dir)
            .into_iter()
            .filter(|(id, _)| ids.iter().any(|wanted| wanted == id))
            .collect();
    if dev_links.is_empty() {
        return Err(anyhow!(
            "no dev-linked sources found for: {}",
            ids.join(", ")
        ));
    }

    let branch = crate::dev::get_active_worktree_branch(&config_dir);
    let sink = NoopEventSink;
    let service = crate::dev::default_build_application_service(&sink);
    let run = service.run(&dev_links, Some(&config_dir), branch.as_deref());

    let failures: Vec<&str> = run
        .results
        .iter()
        .filter(|result| !result.success && !result.skipped)
        .map(|result| result.plugin_id.as_str())
        .collect();
    if failures.is_empty() {
        return Ok(());
    }
    Err(anyhow!("rebuild failed for: {}", failures.join(", ")))
}

#[cfg(feature = "dev")]
fn format_rust_sources(workspace: &std::path::Path) -> Result<()> {
    let output = std::process::Command::new("cargo")
        .args(["fmt", "--all"])
        .current_dir(workspace)
        .output()
        .with_context(|| format!("failed to run cargo fmt in {}", workspace.display()))?;
    if !output.status.success() {
        return Err(anyhow!(
            "cargo fmt exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(feature = "dev")]
fn fix_clippy_lints(workspace: &std::path::Path) -> Result<()> {
    let output = std::process::Command::new("cargo")
        .args([
            "clippy",
            "--fix",
            "--workspace",
            "--all-targets",
            "--allow-dirty",
            "--allow-staged",
        ])
        .env("CARGO_TERM_COLOR", "never")
        .current_dir(workspace)
        .output()
        .with_context(|| {
            format!(
                "failed to run cargo clippy --fix in {}",
                workspace.display()
            )
        })?;
    if !output.status.success() {
        return Err(anyhow!(
            "cargo clippy --fix exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(feature = "dev")]
fn prune_reserved_plugins(ids: &[String]) -> Result<()> {
    remove_registry_entries(ids)
}

#[cfg(feature = "dev")]
fn remove_dev_link_entries(ids: &[String]) -> Result<()> {
    remove_registry_entries(ids)
}

#[cfg(feature = "dev")]
fn remove_registry_entries(ids: &[String]) -> Result<()> {
    let config_dir = crate::paths::shared_config_dir()?;
    let mut registry = crate::plugins::registry::load_registry(&config_dir)
        .map_err(|error| anyhow!("failed to load registry: {error}"))?;
    let before = registry.entries.len();
    registry.entries.retain(|entry| !ids.contains(&entry.id));
    if registry.entries.len() == before {
        return Ok(());
    }
    crate::plugins::registry::save_registry(&config_dir, &registry)
        .map_err(|error| anyhow!("failed to save registry: {error}"))
}

#[cfg(feature = "dev")]
fn prune_orphan_fingerprints(ids: &[String]) -> Result<()> {
    let config_dir = crate::paths::shared_config_dir()?;
    let mut fingerprints = crate::dev::load_build_fingerprints(&config_dir);
    let before = fingerprints.len();
    for id in ids {
        fingerprints.remove(id);
    }
    if fingerprints.len() == before {
        return Ok(());
    }
    crate::dev::save_build_fingerprints(&config_dir, &fingerprints)
        .map_err(|error| anyhow!("failed to save build fingerprints: {error}"))
}

pub(super) trait SymbolicHotkeyWriter {
    fn disable(&mut self, hotkey_id: u32) -> Result<()>;
}

pub(super) fn apply_disable_symbolic_hotkey(
    hotkey_id: u32,
    qol_combo: &str,
    backend: &mut dyn SymbolicHotkeyWriter,
) -> Result<()> {
    if qol_combo.is_empty() {
        return Err(anyhow!("empty qol combo for symbolichotkey {hotkey_id}"));
    }
    backend.disable(hotkey_id)
}

pub(super) trait AppKeyWriter {
    fn clear(&mut self, app_key: &str) -> Result<()>;
}

pub(super) fn apply_clear_windows_app_key(
    app_key: &str,
    qol_combo: &str,
    backend: &mut dyn AppKeyWriter,
) -> Result<()> {
    if qol_combo.is_empty() {
        return Err(anyhow!("empty qol combo for AppKey {app_key}"));
    }
    backend.clear(app_key)
}

pub(super) struct UnshadowRequest<'a> {
    pub dir: &'a str,
    pub key: &'a str,
    pub qol_combo: &'a str,
    pub orphaned: bool,
}

pub(super) trait DeBindingStore {
    fn read(&mut self, dir: &str, key: &str) -> Result<String>;
    fn take_over(&mut self, mutation: &BindingMutation) -> Result<()>;
}

struct DconfTakeover;

impl DeBindingStore for DconfTakeover {
    fn read(&mut self, dir: &str, key: &str) -> Result<String> {
        takeover::read_binding(dir, key).map_err(Into::into)
    }

    fn take_over(&mut self, mutation: &BindingMutation) -> Result<()> {
        takeover::take_over(mutation).map_err(Into::into)
    }
}

pub(super) fn apply_unshadow(
    request: &UnshadowRequest<'_>,
    store: &mut dyn DeBindingStore,
) -> Result<()> {
    let raw = store.read(request.dir, request.key)?;
    let entries = takeover::dconf::parse_string_array(&raw)
        .ok_or_else(|| anyhow!("{}{} is not a keybinding list", request.dir, request.key))?;
    let filtered = filter_unshadow(&entries, request.qol_combo)
        .ok_or_else(|| anyhow!("failed to normalize qol combo: {}", request.qol_combo))?;
    if filtered.len() == entries.len() {
        return Ok(());
    }
    store.take_over(&BindingMutation {
        dir: request.dir.to_string(),
        key: request.key.to_string(),
        next: takeover::dconf::serialize_string_array(&filtered),
        qol_combo: request.qol_combo.to_string(),
        reach: reach_of(request.orphaned),
    })
}

fn reach_of(orphaned: bool) -> BindingReach {
    if orphaned {
        BindingReach::LegacyOrphan
    } else {
        BindingReach::Managed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct StubStore {
        values: BTreeMap<String, String>,
        mutations: RefCell<Vec<BindingMutation>>,
        read_failure: bool,
        write_failure: bool,
    }

    impl StubStore {
        fn with_value(dir: &str, key: &str, value: &str) -> Self {
            let mut values = BTreeMap::new();
            values.insert(format!("{dir}{key}"), value.to_string());
            Self {
                values,
                ..Self::default()
            }
        }

        fn fail_read(mut self) -> Self {
            self.read_failure = true;
            self
        }

        fn fail_write(mut self) -> Self {
            self.write_failure = true;
            self
        }

        fn applied(&self) -> Vec<String> {
            self.mutations
                .borrow()
                .iter()
                .map(|mutation| mutation.next.clone())
                .collect()
        }
    }

    impl DeBindingStore for StubStore {
        fn read(&mut self, dir: &str, key: &str) -> Result<String> {
            if self.read_failure {
                return Err(anyhow!("read failed"));
            }
            self.values
                .get(&format!("{dir}{key}"))
                .cloned()
                .ok_or_else(|| anyhow!("missing entry"))
        }

        fn take_over(&mut self, mutation: &BindingMutation) -> Result<()> {
            if self.write_failure {
                return Err(anyhow!("write failed"));
            }
            self.mutations.borrow_mut().push(mutation.clone());
            Ok(())
        }
    }

    fn request<'a>(dir: &'a str, key: &'a str, combo: &'a str) -> UnshadowRequest<'a> {
        UnshadowRequest {
            dir,
            key,
            qol_combo: combo,
            orphaned: false,
        }
    }

    #[test]
    fn safe_automatic_orders_below_reversible_host_mutation() {
        assert!(FixApplicability::SafeAutomatic < FixApplicability::ReversibleHostMutation);
    }

    #[cfg(feature = "dev")]
    #[test]
    fn relocate_dev_link_is_safe_automatic() {
        let action = FixAction::RelocateDevLink {
            plugin_id: "plugin-foo".into(),
            to: PathBuf::from("/ws/plugins/plugin-foo"),
        };
        assert_eq!(action.applicability(), FixApplicability::SafeAutomatic);
    }

    #[cfg(feature = "dev")]
    #[test]
    fn prune_orphan_fingerprints_is_safe_automatic() {
        let action = FixAction::PruneOrphanFingerprints {
            ids: vec!["plugin-orphan".into()],
        };
        assert_eq!(action.applicability(), FixApplicability::SafeAutomatic);
    }

    #[cfg(feature = "dev")]
    #[test]
    fn prune_reserved_plugins_is_safe_automatic() {
        let action = FixAction::PruneReservedPlugins {
            ids: vec!["plugin-template".into()],
        };
        assert_eq!(action.applicability(), FixApplicability::SafeAutomatic);
    }

    #[cfg(feature = "dev")]
    #[test]
    fn format_rust_sources_is_safe_automatic() {
        let action = FixAction::FormatRustSources {
            workspace: PathBuf::from("/ws"),
        };
        assert_eq!(action.applicability(), FixApplicability::SafeAutomatic);
    }

    #[cfg(feature = "dev")]
    #[test]
    fn fix_clippy_lints_is_safe_automatic() {
        let action = FixAction::FixClippyLints {
            workspace: PathBuf::from("/ws"),
        };
        assert_eq!(action.applicability(), FixApplicability::SafeAutomatic);
    }

    #[cfg(feature = "dev")]
    #[test]
    fn prune_cargo_incremental_cache_is_safe_automatic() {
        let action = FixAction::PruneCargoIncrementalCache {
            path: PathBuf::from("/ws/target/debug/incremental"),
        };
        assert_eq!(action.applicability(), FixApplicability::SafeAutomatic);
    }

    #[cfg(feature = "dev")]
    #[test]
    fn prune_cargo_incremental_cache_removes_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = dir.path().join("target").join("debug").join("incremental");
        std::fs::create_dir_all(&cache).expect("cache dir");
        std::fs::write(cache.join("dep-graph.bin"), b"x").expect("cache file");

        apply_fix(&FixAction::PruneCargoIncrementalCache {
            path: cache.clone(),
        })
        .expect("prune cache");

        assert!(!cache.exists());
    }

    #[test]
    fn applicability_maps_each_action() {
        let cases = [
            (
                FixAction::SetActiveInstallId("abc".into()),
                FixApplicability::SafeAutomatic,
            ),
            (
                FixAction::WriteInstallMarker {
                    marker_path: PathBuf::from("/tmp/x"),
                    install_id: "abc".into(),
                },
                FixApplicability::SafeAutomatic,
            ),
            (
                FixAction::WriteAutostartEntry {
                    binary_path: PathBuf::from("/usr/bin/qol-tray"),
                },
                FixApplicability::SafeAutomatic,
            ),
            (
                FixAction::EnsurePluginsDir {
                    path: PathBuf::from("/tmp/plugins"),
                },
                FixApplicability::SafeAutomatic,
            ),
            (
                FixAction::KillPluginProcessLeaks {
                    processes: Vec::new(),
                },
                FixApplicability::SafeAutomatic,
            ),
            (FixAction::InstallShellHook, FixApplicability::SafeAutomatic),
            (
                FixAction::DrainOrphanPluginConfigs,
                FixApplicability::SafeAutomatic,
            ),
            (
                FixAction::UnshadowDeBinding {
                    dir: "org.cinnamon.desktop.keybindings.wm".into(),
                    key: "switch-input-source".into(),
                    qol_combo: "Super+Space".into(),
                    orphaned: false,
                },
                FixApplicability::ReversibleHostMutation,
            ),
            (
                FixAction::DisableSymbolicHotkey {
                    hotkey_id: 64,
                    qol_combo: "Cmd+Space".into(),
                },
                FixApplicability::ReversibleHostMutation,
            ),
            (
                FixAction::ClearWindowsAppKey {
                    app_key: "17".into(),
                    qol_combo: "Win+E".into(),
                },
                FixApplicability::ReversibleHostMutation,
            ),
            (
                FixAction::HoldNvidiaDriverPackages,
                FixApplicability::ManualOnly,
            ),
            (
                FixAction::UnholdNvidiaDriverPackages,
                FixApplicability::ManualOnly,
            ),
            (
                FixAction::ApplyHeldNvidiaDriverUpdate {
                    packages: vec!["nvidia-driver-560".into()],
                },
                FixApplicability::ManualOnly,
            ),
        ];
        for (action, expected) in cases {
            assert_eq!(
                action.applicability(),
                expected,
                "action variant: {:?}",
                std::mem::discriminant(&action)
            );
        }
    }

    struct StubSymbolicWriter {
        disabled: RefCell<Vec<u32>>,
        fail_on: Option<u32>,
    }

    impl StubSymbolicWriter {
        fn new() -> Self {
            Self {
                disabled: RefCell::new(Vec::new()),
                fail_on: None,
            }
        }

        fn fail_on(mut self, id: u32) -> Self {
            self.fail_on = Some(id);
            self
        }
    }

    impl SymbolicHotkeyWriter for StubSymbolicWriter {
        fn disable(&mut self, hotkey_id: u32) -> Result<()> {
            if Some(hotkey_id) == self.fail_on {
                return Err(anyhow!("disable failed"));
            }
            self.disabled.borrow_mut().push(hotkey_id);
            Ok(())
        }
    }

    #[test]
    fn apply_disable_symbolic_hotkey_writes_through_to_backend() {
        let mut backend = StubSymbolicWriter::new();
        apply_disable_symbolic_hotkey(64, "Cmd+Space", &mut backend).expect("ok");
        apply_disable_symbolic_hotkey(60, "Ctrl+Space", &mut backend).expect("ok");
        assert_eq!(backend.disabled.borrow().as_slice(), &[64, 60]);
    }

    #[test]
    fn apply_disable_symbolic_hotkey_propagates_backend_failure() {
        let mut backend = StubSymbolicWriter::new().fail_on(64);
        let err = apply_disable_symbolic_hotkey(64, "Cmd+Space", &mut backend)
            .expect_err("backend failure must propagate");
        assert_eq!(err.to_string(), "disable failed");
        assert!(backend.disabled.borrow().is_empty());
    }

    #[test]
    fn apply_disable_symbolic_hotkey_rejects_empty_combo() {
        let mut backend = StubSymbolicWriter::new();
        let err = apply_disable_symbolic_hotkey(64, "", &mut backend)
            .expect_err("empty combo must be rejected");
        assert!(err.to_string().contains("empty qol combo"));
        assert!(
            backend.disabled.borrow().is_empty(),
            "backend must not be called for empty combo"
        );
    }

    struct StubAppKeyWriter {
        cleared: RefCell<Vec<String>>,
        fail_on: Option<String>,
    }

    impl StubAppKeyWriter {
        fn new() -> Self {
            Self {
                cleared: RefCell::new(Vec::new()),
                fail_on: None,
            }
        }

        fn fail_on(mut self, key: &str) -> Self {
            self.fail_on = Some(key.to_string());
            self
        }
    }

    impl AppKeyWriter for StubAppKeyWriter {
        fn clear(&mut self, app_key: &str) -> Result<()> {
            if self.fail_on.as_deref() == Some(app_key) {
                return Err(anyhow!("clear failed"));
            }
            self.cleared.borrow_mut().push(app_key.to_string());
            Ok(())
        }
    }

    #[test]
    fn apply_clear_windows_app_key_round_trip_cases() {
        let cases = [("17", "Win+E", true), ("18", "Win+Q", true)];
        let mut backend = StubAppKeyWriter::new();
        for (key, combo, expected_ok) in cases {
            let result = apply_clear_windows_app_key(key, combo, &mut backend);
            assert_eq!(result.is_ok(), expected_ok, "key={key}, combo={combo}");
        }
        assert_eq!(
            backend.cleared.borrow().as_slice(),
            &["17".to_string(), "18".to_string()]
        );
    }

    #[test]
    fn apply_clear_windows_app_key_propagates_backend_failure() {
        let mut backend = StubAppKeyWriter::new().fail_on("17");
        let err = apply_clear_windows_app_key("17", "Win+E", &mut backend)
            .expect_err("backend failure must propagate");
        assert_eq!(err.to_string(), "clear failed");
    }

    #[test]
    fn apply_clear_windows_app_key_rejects_empty_combo() {
        let mut backend = StubAppKeyWriter::new();
        let err = apply_clear_windows_app_key("17", "", &mut backend)
            .expect_err("empty combo must be rejected");
        assert!(err.to_string().contains("empty qol combo"));
    }

    #[test]
    fn apply_unshadow_removes_only_the_conflicting_entry() {
        let dir = "/org/cinnamon/desktop/keybindings/wm/";
        let key = "switch-input-source";
        let mut store = StubStore::with_value(dir, key, "['<Super>space', 'XF86Keyboard']");
        apply_unshadow(&request(dir, key, "Super+Space"), &mut store).expect("apply ok");
        assert_eq!(store.applied(), vec!["['XF86Keyboard']".to_string()]);
    }

    #[test]
    fn apply_unshadow_writes_a_typed_empty_array_when_only_the_conflict_is_present() {
        let dir = "/desktop/ibus/general/hotkey/";
        let key = "triggers";
        let mut store = StubStore::with_value(dir, key, "['<Super>space']");
        apply_unshadow(&request(dir, key, "Super+Space"), &mut store).expect("apply ok");
        assert_eq!(
            store.applied(),
            vec!["@as []".to_string()],
            "dconf write rejects the untyped [] literal"
        );
    }

    #[test]
    fn apply_unshadow_is_a_no_op_when_nothing_overlaps() {
        let dir = "/org/cinnamon/desktop/keybindings/wm/";
        let key = "panel-main-menu";
        let mut store = StubStore::with_value(dir, key, "['<Super>r','<Alt>F2','XF86Keyboard']");
        apply_unshadow(&request(dir, key, "Super+Space"), &mut store).expect("apply ok");
        assert!(
            store.applied().is_empty(),
            "a no-op must not record a takeover claim the host would later restore"
        );
    }

    #[test]
    fn apply_unshadow_carries_the_orphan_flag_into_the_recorded_mutation() {
        let dir = "/org/cinnamon/desktop/keybindings/custom2/";
        let key = "binding";
        let cases = [
            (true, BindingReach::LegacyOrphan),
            (false, BindingReach::Managed),
        ];
        for (orphaned, expected) in cases {
            let mut store = StubStore::with_value(dir, key, "['<Shift><Super>s']");
            apply_unshadow(
                &UnshadowRequest {
                    dir,
                    key,
                    qol_combo: "Shift+Super+S",
                    orphaned,
                },
                &mut store,
            )
            .expect("apply ok");
            let mutations = store.mutations.borrow();
            assert_eq!(mutations.len(), 1, "orphaned: {orphaned}");
            assert_eq!(mutations[0].reach, expected, "orphaned: {orphaned}");
            assert_eq!(mutations[0].qol_combo, "Shift+Super+S");
        }
    }

    #[test]
    fn apply_unshadow_rejects_values_that_are_not_keybinding_lists() {
        let dir = "/org/cinnamon/desktop/keybindings/custom2/";
        let key = "command";
        let mut store = StubStore::with_value(dir, key, "'flameshot gui'");
        let err = apply_unshadow(&request(dir, key, "Super+Space"), &mut store)
            .expect_err("a string value must not be rewritten as a list");
        assert!(
            err.to_string().contains("is not a keybinding list"),
            "{err}"
        );
        assert!(store.applied().is_empty());
    }

    #[test]
    fn apply_unshadow_returns_err_for_unparseable_qol_combo() {
        let dir = "/org/cinnamon/desktop/keybindings/wm/";
        let key = "switch-input-source";
        let mut store = StubStore::with_value(dir, key, "['<Super>space']");
        let err = apply_unshadow(&request(dir, key, "<Super>"), &mut store)
            .expect_err("should reject unnormalizable combo");
        assert!(
            err.to_string().contains("failed to normalize qol combo"),
            "actual: {err}"
        );
        assert!(store.applied().is_empty());
    }

    #[test]
    fn apply_unshadow_propagates_read_and_write_failures_without_recording_a_claim() {
        let dir = "/org/cinnamon/desktop/keybindings/wm/";
        let key = "switch-input-source";
        let cases = [
            (
                StubStore::with_value(dir, key, "['<Super>space']").fail_read(),
                "read failed",
            ),
            (
                StubStore::with_value(dir, key, "['<Super>space']").fail_write(),
                "write failed",
            ),
        ];
        for (mut store, expected) in cases {
            let err = apply_unshadow(&request(dir, key, "Super+Space"), &mut store)
                .expect_err("failure must propagate");
            assert_eq!(err.to_string(), expected);
            assert!(store.applied().is_empty());
        }
    }
}
