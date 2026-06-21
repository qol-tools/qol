use qol_tray::features::launcher_apps;
use qol_tray::hotkeys::{HotkeyBinding, HotkeyConfig, HotkeyManager};
use qol_tray::shortcuts::model::{AppRef, Shortcut, ShortcutAction, ShortcutsConfig};
use qol_tray::shortcuts::store as shortcut_store;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tempfile::TempDir;
use tokio::sync::Mutex;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvContext {
    test_path_root: Option<OsString>,
    home: Option<OsString>,
    xdg_config_home: Option<OsString>,
}

impl EnvContext {
    fn new(root_dir: &Path, home_dir: &Path, xdg_config_dir: &Path) -> Self {
        let test_path_root = std::env::var_os("QOL_TRAY_TEST_PATH_ROOT");
        let home = std::env::var_os("HOME");
        let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("QOL_TRAY_TEST_PATH_ROOT", root_dir);
        std::env::set_var("HOME", home_dir);
        std::env::set_var("XDG_CONFIG_HOME", xdg_config_dir);
        Self {
            test_path_root,
            home,
            xdg_config_home,
        }
    }
}

impl Drop for EnvContext {
    fn drop(&mut self) {
        match &self.test_path_root {
            Some(value) => std::env::set_var("QOL_TRAY_TEST_PATH_ROOT", value),
            None => std::env::remove_var("QOL_TRAY_TEST_PATH_ROOT"),
        }
        match &self.home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match &self.xdg_config_home {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}

struct TestEnv {
    _root: TempDir,
    root_dir: PathBuf,
    home_dir: PathBuf,
    xdg_config_dir: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let root_dir = root.path().to_path_buf();
        let home_dir = root.path().join("home");
        let xdg_config_dir = root.path().join("xdg-config");
        fs::create_dir_all(&home_dir).unwrap();
        fs::create_dir_all(&xdg_config_dir).unwrap();
        Self {
            _root: root,
            root_dir,
            home_dir,
            xdg_config_dir,
        }
    }

    fn enter(&self) -> EnvContext {
        EnvContext::new(&self.root_dir, &self.home_dir, &self.xdg_config_dir)
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_src(relative: &str) -> String {
    let path = workspace_root().join("src").join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e))
}

fn enabled_binding(id: &str, key: &str, plugin_id: &str, action: &str) -> HotkeyBinding {
    HotkeyBinding {
        id: id.to_string(),
        key: key.to_string(),
        plugin_id: plugin_id.to_string(),
        action: action.to_string(),
        enabled: true,
    }
}

fn disabled_binding(id: &str, key: &str, plugin_id: &str, action: &str) -> HotkeyBinding {
    HotkeyBinding {
        id: id.to_string(),
        key: key.to_string(),
        plugin_id: plugin_id.to_string(),
        action: action.to_string(),
        enabled: false,
    }
}

fn url_shortcut(id: &str, name: &str, enabled: bool, export: bool, url: &str) -> Shortcut {
    Shortcut {
        id: id.to_string(),
        name: name.to_string(),
        enabled,
        export_to_launcher: export,
        source: None,
        action: ShortcutAction::OpenUrl {
            url: url.to_string(),
            browser_override: None,
        },
    }
}

fn launch_app_shortcut(id: &str, name: &str, bundle: &str) -> Shortcut {
    Shortcut {
        id: id.to_string(),
        name: name.to_string(),
        enabled: true,
        export_to_launcher: true,
        source: None,
        action: ShortcutAction::LaunchApp {
            app: AppRef::BundleId {
                id: bundle.to_string(),
            },
        },
    }
}

fn entry_stems(entries: &[launcher_apps::LauncherEntry]) -> Vec<String> {
    entries.iter().map(|e| e.file_stem.clone()).collect()
}

#[tokio::test(flavor = "current_thread")]
async fn hotkey_save_round_trips_through_public_manager_api() {
    let _guard = env_lock().lock().await;
    let env = TestEnv::new();
    let _ctx = env.enter();
    qol_tray::profile::ensure_profile_dirs().unwrap();

    let manager = HotkeyManager::new().unwrap();
    let original = HotkeyConfig {
        hotkeys: vec![
            enabled_binding("a", "Ctrl+1", "plugin-a", "do-a"),
            disabled_binding("b", "Ctrl+2", "plugin-b", "do-b"),
            enabled_binding("c", "Ctrl+Shift+F1", "plugin-c", "do-c"),
        ],
    };
    manager.save_config(&original).unwrap();

    let reloaded = manager.load_config().unwrap();
    let observed: Vec<(&str, &str, &str, bool)> = reloaded
        .hotkeys
        .iter()
        .map(|h| (h.id.as_str(), h.key.as_str(), h.action.as_str(), h.enabled))
        .collect();
    assert_eq!(
        observed,
        vec![
            ("a", "Ctrl+1", "do-a", true),
            ("b", "Ctrl+2", "do-b", false),
            ("c", "Ctrl+Shift+F1", "do-c", true),
        ],
        "saved config must round-trip through the public manager API in order"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn shortcut_add_update_remove_reflects_in_derived_launcher_entry_set() {
    let _guard = env_lock().lock().await;
    let env = TestEnv::new();
    let _ctx = env.enter();
    qol_tray::profile::ensure_profile_dirs().unwrap();

    let mut config = ShortcutsConfig::default();
    shortcut_store::add(
        &mut config,
        url_shortcut("alpha", "Alpha", true, true, "https://alpha.example"),
    )
    .unwrap();
    shortcut_store::add(
        &mut config,
        url_shortcut("beta", "Beta", true, false, "https://beta.example"),
    )
    .unwrap();
    shortcut_store::add(
        &mut config,
        url_shortcut("gamma", "Gamma", false, true, "https://gamma.example"),
    )
    .unwrap();
    shortcut_store::add(&mut config, launch_app_shortcut("delta", "Delta", "com.x")).unwrap();
    shortcut_store::save(&config).unwrap();

    let after_add = shortcut_store::load().unwrap();
    let after_add_entries = launcher_apps::collect_shortcut_entries(&after_add.shortcuts);
    assert_eq!(
        entry_stems(&after_add_entries),
        vec!["shortcut-alpha".to_string(), "shortcut-delta".to_string()],
        "only enabled + exported shortcuts must appear as launcher entries"
    );

    shortcut_store::update(
        &mut config,
        url_shortcut("beta", "Beta", true, true, "https://beta.example"),
    )
    .unwrap();
    shortcut_store::save(&config).unwrap();
    let after_enable_export =
        launcher_apps::collect_shortcut_entries(&shortcut_store::load().unwrap().shortcuts);
    assert_eq!(
        entry_stems(&after_enable_export),
        vec![
            "shortcut-alpha".to_string(),
            "shortcut-beta".to_string(),
            "shortcut-delta".to_string(),
        ],
        "flipping export_to_launcher must add the entry on next derive"
    );

    shortcut_store::update(
        &mut config,
        url_shortcut("alpha", "Alpha", false, true, "https://alpha.example"),
    )
    .unwrap();
    shortcut_store::save(&config).unwrap();
    let after_disable =
        launcher_apps::collect_shortcut_entries(&shortcut_store::load().unwrap().shortcuts);
    assert_eq!(
        entry_stems(&after_disable),
        vec!["shortcut-beta".to_string(), "shortcut-delta".to_string()],
        "disabling a shortcut must drop it from launcher entries on next derive"
    );

    shortcut_store::remove(&mut config, "delta").unwrap();
    shortcut_store::save(&config).unwrap();
    let after_remove =
        launcher_apps::collect_shortcut_entries(&shortcut_store::load().unwrap().shortcuts);
    assert_eq!(
        entry_stems(&after_remove),
        vec!["shortcut-beta".to_string()],
        "removing a shortcut must drop it from launcher entries on next derive"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn shortcut_exec_args_carry_id_for_each_derived_entry() {
    let _guard = env_lock().lock().await;
    let env = TestEnv::new();
    let _ctx = env.enter();
    qol_tray::profile::ensure_profile_dirs().unwrap();

    let mut config = ShortcutsConfig::default();
    shortcut_store::add(
        &mut config,
        url_shortcut("docs", "Docs", true, true, "https://docs.example"),
    )
    .unwrap();
    shortcut_store::add(
        &mut config,
        launch_app_shortcut("safari", "Safari", "com.apple.Safari"),
    )
    .unwrap();
    shortcut_store::save(&config).unwrap();

    let entries =
        launcher_apps::collect_shortcut_entries(&shortcut_store::load().unwrap().shortcuts);

    let cases = [
        ("shortcut-docs", vec!["exec", "shortcut", "docs"]),
        ("shortcut-safari", vec!["exec", "shortcut", "safari"]),
    ];
    for (stem, expected_args) in cases {
        let entry = entries
            .iter()
            .find(|e| e.file_stem == stem)
            .unwrap_or_else(|| panic!("entry {stem} missing"));
        assert_eq!(
            entry.exec_args, expected_args,
            "exec_args for {stem} must encode the shortcut id"
        );
        assert_eq!(
            entry.bundle_id,
            format!("com.qol-tools.shortcut.{}", &stem["shortcut-".len()..]),
            "bundle_id for {stem} must be derived from the shortcut id"
        );
    }
}

mod write_point_dispatch_contracts {
    use super::read_src;

    #[test]
    fn hotkey_save_handler_emits_config_changed_after_save_and_does_not_drive_backend_directly() {
        let src = read_src("features/plugin_store/server/settings/hotkey_handlers.rs");
        let save_at = src
            .find(".save_config(&config)")
            .expect("set_hotkeys_inner must persist via save_config before signaling");
        let signal_at = src.find("config_changed(ConfigKind::Hotkeys)").expect(
            "set_hotkeys_inner must emit ConfigChanged{Hotkeys} after save so the \
                 reconciler re-derives the live capture matcher",
        );
        assert!(
            save_at < signal_at,
            "ConfigChanged{{Hotkeys}} must follow save_config so persisted bindings are what \
             gets re-derived; reversing would race a stale config into the matcher"
        );
        assert!(
            !src.contains("trigger_reload()"),
            "set_hotkeys_inner must NOT drive hotkeys::trigger_reload() directly anymore - \
             reconcile is now an independent subscriber of ConfigChanged{{Hotkeys}}"
        );
    }

    #[test]
    fn shortcut_handlers_emit_config_changed_shortcuts_on_every_mutation() {
        let src = read_src("features/plugin_store/server/settings/shortcut_handlers.rs");
        assert_eq!(
            src.matches("config_changed(ConfigKind::Shortcuts)").count(),
            3,
            "create, update and delete handlers must each emit ConfigChanged{{Shortcuts}}; \
             count drift signals a missing reconcile path"
        );
        assert!(
            !src.contains("trigger_launcher_sync"),
            "shortcut handlers must NOT drive launcher sync directly anymore - reconcile is \
             now an independent subscriber of ConfigChanged{{Shortcuts}}"
        );
        assert!(
            !src.contains("trigger_full_sync_with_manager"),
            "shortcut handlers must NOT reach into launcher_apps directly"
        );
    }

    #[test]
    fn plugin_install_uninstall_update_all_go_through_reload_manager_and_notify() {
        let cases = [
            "features/plugin_store/server/plugin_services/operations/install.rs",
            "features/plugin_store/server/plugin_services/operations/uninstall.rs",
            "features/plugin_store/server/plugin_services/operations/update.rs",
        ];
        for relative in cases {
            let src = read_src(relative);
            assert!(
                src.contains("reload_manager_and_notify(state)"),
                "{relative} must call reload_manager_and_notify(state) so menu, hotkeys, \
                 launcher entries and SSE subscribers all reconcile after the operation"
            );
        }
    }

    #[test]
    fn reload_manager_and_notify_emits_one_plugins_signal_keeps_plugins_changed_and_runs_reload() {
        let src = read_src("features/plugin_store/server/helpers.rs");
        let helper_signals = [
            "manager.reload_plugins()",
            "config_changed(ConfigKind::Plugins)",
            "state.daemon.events.send_plugins_changed()",
        ];
        for signal in helper_signals {
            assert!(
                src.contains(signal),
                "reload_manager_and_notify must invoke `{signal}` so plugin set + UI both \
                 reconcile after the operation"
            );
        }
        assert!(
            !src.contains("trigger_full_sync_with_plugins"),
            "helpers.rs must NOT drive launcher sync directly; reconciler subscribes instead"
        );
        assert!(
            !src.contains("trigger_reload()"),
            "helpers.rs must NOT drive hotkeys::trigger_reload() directly; reconciler \
             subscribes instead"
        );
    }

    #[test]
    fn plugin_config_save_handler_only_reloads_the_owning_plugin_daemon() {
        let handler =
            read_src("features/plugin_store/server/settings/plugin_config_handlers/mod.rs");
        assert!(
            handler.contains("notify::notify_plugin_reload(state, &plugin_id)"),
            "set_plugin_config_inner must invoke notify_plugin_reload after save so the live \
             daemon picks up the new config"
        );
        assert!(
            !handler.contains("trigger_reload()"),
            "set_plugin_config_inner is currently scoped to the daemon only - it does NOT \
             call hotkeys::trigger_reload(); flip this when config changes can rebind hotkeys"
        );
        assert!(
            !handler.contains("trigger_full_sync"),
            "set_plugin_config_inner does NOT trigger launcher sync today; flip this when \
             config edits can change exported shortcut surface"
        );
        assert!(
            !handler.contains("config_changed("),
            "set_plugin_config_inner does NOT emit a host-level ConfigChanged signal today; \
             plugin config edits route to the owning daemon only"
        );
        assert!(
            !handler.contains("send_plugins_changed"),
            "set_plugin_config_inner does NOT broadcast PluginsChanged today; flip this when \
             the menu or store UI must rebuild on config edits"
        );

        let notify =
            read_src("features/plugin_store/server/settings/plugin_config_handlers/notify.rs");
        assert!(
            notify.contains("platform::notify_plugin_reload(socket_path)"),
            "notify::notify_plugin_reload must reach the plugin's socket before falling back"
        );
        assert!(
            notify.contains("restart_running_plugin_daemon"),
            "notify::notify_plugin_reload must restart the daemon if socket notify fails"
        );
    }
}

mod profile_apply_reconciles_via_config_changed_signal {
    use super::read_src;

    #[test]
    fn reload_after_profile_apply_emits_profile_signal_and_keeps_plugins_changed() {
        let src = read_src("features/profile/http/mod.rs");
        let expected_signals = [
            "manager.reload_plugins_if_changed()",
            "config_changed(crate::daemon::ConfigKind::Profile)",
            "state.daemon.events.send_plugins_changed()",
        ];
        for signal in expected_signals {
            assert!(
                src.contains(signal),
                "reload_after_profile_apply must invoke `{signal}` so profile import / pull / \
                 switch fans out via the ConfigChanged{{Profile}} signal + UI broadcast"
            );
        }
        assert!(
            !src.contains("trigger_full_sync_with_plugins"),
            "profile apply must NOT drive launcher sync directly anymore - reconciler \
             subscribes to ConfigChanged{{Profile}}"
        );
        assert!(
            !src.contains("crate::hotkeys::trigger_reload()"),
            "profile apply must NOT drive hotkeys reload directly anymore - reconciler \
             subscribes to ConfigChanged{{Profile}}"
        );
    }
}

mod boot_wires_independent_reconcilers {
    use super::read_src;

    #[test]
    fn main_spawns_one_reconciler_for_hotkeys_and_one_for_launcher_apps() {
        let src = read_src("main.rs");
        assert!(
            src.contains("spawn_config_reconcilers"),
            "app_init_inner must spawn the config reconcilers at boot via \
             spawn_config_reconcilers"
        );
        let body = src
            .split_once("fn spawn_config_reconcilers")
            .map(|(_, rest)| rest)
            .expect("spawn_config_reconcilers must be defined in main.rs");
        assert!(
            body.contains("ConfigKind::Hotkeys")
                && body.contains("ConfigKind::Plugins")
                && body.contains("ConfigKind::Profile"),
            "the hotkeys reconciler must subscribe to Hotkeys + Plugins + Profile so \
             hotkeys reload after a config save, a plugin set change, or a profile apply"
        );
        assert!(
            body.contains("hotkeys::trigger_reload"),
            "the hotkeys reconciler must drive hotkeys::trigger_reload() to preserve the \
             211e7fc2 fix (live capture thread drains a crossbeam channel)"
        );
        assert!(
            body.contains("ConfigKind::Shortcuts"),
            "the launcher reconciler must subscribe to Shortcuts so shortcut mutations \
             refresh launcher entries"
        );
        assert!(
            body.contains("trigger_full_sync_with_manager"),
            "the launcher reconciler must drive launcher_apps::trigger_full_sync_with_manager"
        );
    }
}

mod tray_menu_gap_pin {
    use super::{read_src, workspace_root};

    #[test]
    fn build_menu_is_only_called_from_tray_platform_constructors() {
        let pattern = "menu::builder::build_menu(";
        let allowed = [
            "tray/platform/linux.rs",
            "tray/platform/macos.rs",
            "tray/platform/windows.rs",
        ];

        let mut callers: Vec<String> = Vec::new();
        let walker = walkdir::WalkDir::new(workspace_root().join("src"))
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rs"));
        for entry in walker {
            let body = std::fs::read_to_string(entry.path()).unwrap_or_default();
            if !body.contains(pattern) {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(workspace_root().join("src"))
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if relative == "menu/builder.rs" {
                continue;
            }
            callers.push(relative);
        }
        callers.sort();
        let mut expected: Vec<String> = allowed.iter().map(|s| s.to_string()).collect();
        expected.sort();
        assert_eq!(
            callers, expected,
            "build_menu must only be invoked by the tray platform constructors; \
             every other caller introduces a rebuild path that is currently unsupported"
        );
    }

    #[test]
    fn no_runtime_rebuild_path_invokes_set_menu_on_tray_icon() {
        let walker = walkdir::WalkDir::new(workspace_root().join("src"))
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rs"));
        let mut offenders = Vec::new();
        for entry in walker {
            let body = std::fs::read_to_string(entry.path()).unwrap_or_default();
            if body.contains(".set_menu(") {
                offenders.push(entry.path().display().to_string());
            }
        }
        assert!(
            offenders.is_empty(),
            "no tray rebuild path may call TrayIcon::set_menu today; found: {:?}. \
             Currently the tray menu is built once at boot from FeatureRegistry::features() \
             and there is no live rebuild on config changes - this test pins that gap.",
            offenders
        );
    }

    #[test]
    fn tray_manager_constructor_takes_feature_registry_once_and_holds_built_tray() {
        let src = read_src("tray/mod.rs");
        assert!(
            src.contains("pub struct TrayManager") && src.contains("_tray: platform::PlatformTray"),
            "TrayManager owns a single PlatformTray built at construction; there is no \
             public hook to swap or rebuild its menu after the fact"
        );
        let menu_mod = read_src("menu/builder.rs");
        assert!(
            menu_mod.contains("pub fn build_menu"),
            "build_menu remains the single entrypoint; if a second rebuild API is added, \
             update this characterization and wire menu re-emission on PluginsChanged"
        );
    }

    #[test]
    fn menu_provider_is_the_only_source_of_menu_items_from_features() {
        let registry = read_src("features/mod.rs");
        assert!(
            registry.contains("fn menu_items(&self) -> Vec<PluginMenuItem>"),
            "MenuProvider::menu_items is the sole derive seam for the menu"
        );
        let builder = read_src("menu/builder.rs");
        assert!(
            builder.contains("feature.menu_items()"),
            "build_menu iterates FeatureRegistry::features() and reads each MenuProvider; \
             nothing else materializes menu items"
        );
    }
}

mod autostart_re_runnable_via_reload {
    use super::read_src;

    #[test]
    fn reload_plugins_calls_autostart_daemons_after_reload() {
        let src = read_src("plugins/manager/runtime.rs");
        assert!(
            src.contains("manager.autostart_daemons()"),
            "PluginManager::reload_plugins must re-run autostart_daemons so daemon-enabled \
             plugins come back up after the reload that kills them all"
        );
        let order_ok = src
            .find("loading::load_plugins(manager)")
            .and_then(|load_at| {
                src.find("manager.autostart_daemons()")
                    .map(|auto_at| auto_at > load_at)
            });
        assert_eq!(
            order_ok,
            Some(true),
            "autostart_daemons() must follow load_plugins(manager); reversing the order \
             would autostart against the pre-reload plugin set"
        );
    }

    #[test]
    fn manager_exposes_autostart_daemons_for_boot_and_reload_paths() {
        let manager = read_src("plugins/manager/mod.rs");
        assert!(
            manager.contains("pub fn autostart_daemons"),
            "autostart_daemons must be pub so both the boot path and reload path can call it"
        );
        assert!(
            manager.contains("pub fn reload_plugins"),
            "reload_plugins must be pub so HTTP handlers can drive a full reconcile"
        );
    }
}

mod spawn_reconciler_drives_subscribers_on_config_changed {
    use qol_tray::daemon::{ConfigBus, ConfigKind};
    use qol_tray::reconcile::spawn_reconciler;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;

    fn counter() -> (Arc<AtomicUsize>, impl Fn() + Send + Sync + 'static) {
        let counter = Arc::new(AtomicUsize::new(0));
        let cloned = Arc::clone(&counter);
        (counter, move || {
            cloned.fetch_add(1, Ordering::SeqCst);
        })
    }

    async fn wait_at_least(counter: &AtomicUsize, expected: usize) {
        for _ in 0..100 {
            if counter.load(Ordering::SeqCst) >= expected {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "timed out waiting for reconciler count {} (observed {})",
            expected,
            counter.load(Ordering::SeqCst)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn matching_interest_fires_reconciler_and_non_matching_does_not() {
        let bus = ConfigBus::new();
        let (count, work) = counter();
        spawn_reconciler(&bus, &[ConfigKind::Hotkeys], work);
        sleep(Duration::from_millis(20)).await;

        bus.config_changed(ConfigKind::Shortcuts);
        bus.config_changed(ConfigKind::Plugins);
        bus.config_changed(ConfigKind::Profile);
        sleep(Duration::from_millis(50)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "non-matching config kinds must not fire the reconciler"
        );

        bus.config_changed(ConfigKind::Hotkeys);
        wait_at_least(&count, 1).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn burst_of_matching_signals_coalesces_into_fewer_reconciles() {
        let bus = ConfigBus::new();
        let (count, work) = counter();
        spawn_reconciler(&bus, &[ConfigKind::Plugins], work);
        sleep(Duration::from_millis(20)).await;

        for _ in 0..10 {
            bus.config_changed(ConfigKind::Plugins);
        }
        wait_at_least(&count, 1).await;
        sleep(Duration::from_millis(60)).await;
        let observed = count.load(Ordering::SeqCst);
        assert!(
            observed < 10,
            "burst of 10 matching events must coalesce into fewer reconciles; observed {}",
            observed
        );
    }
}

mod config_signal_is_isolated_from_the_ui_bus {
    use qol_tray::daemon::{ConfigKind, Daemon};
    use std::time::Duration;
    use tokio::sync::broadcast::error::TryRecvError;
    use tokio::time::sleep;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn config_signal_does_not_reach_the_ui_event_bus() {
        let daemon = Daemon::new();
        let mut ui = daemon.events.subscribe();
        daemon.config.config_changed(ConfigKind::Hotkeys);
        sleep(Duration::from_millis(20)).await;
        assert!(
            matches!(ui.try_recv(), Err(TryRecvError::Empty)),
            "config reconcile signals must not leak onto the UI event/SSE bus"
        );
    }
}
