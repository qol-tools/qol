use qol_tray::paths;
use qol_tray::profile::{self, ProfileExportBundle, ProfileImportBundle};
use qol_tray::sync::{SyncConnectRequest, SyncHealth, SyncService};
use serde_json::{json, Value};
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

struct TestEnv {
    _root: TempDir,
    home_dir: PathBuf,
    xdg_config_dir: PathBuf,
    plugins_dir: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let home_dir = root.path().join("home");
        let xdg_config_dir = root.path().join("xdg-config");
        fs::create_dir_all(&home_dir).unwrap();
        fs::create_dir_all(&xdg_config_dir).unwrap();

        let _ctx = EnvContext::new(&home_dir, &xdg_config_dir);
        let config_dir = paths::shared_config_dir().unwrap();
        let plugins_dir = config_dir.join("plugins");
        fs::create_dir_all(&plugins_dir).unwrap();

        Self {
            _root: root,
            home_dir,
            xdg_config_dir,
            plugins_dir,
        }
    }

    fn enter(&self) -> EnvContext {
        EnvContext::new(&self.home_dir, &self.xdg_config_dir)
    }
}

struct EnvContext {
    home: Option<OsString>,
    xdg_config_home: Option<OsString>,
}

impl EnvContext {
    fn new(home_dir: &Path, xdg_config_dir: &Path) -> Self {
        let home = std::env::var_os("HOME");
        let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("HOME", home_dir);
        std::env::set_var("XDG_CONFIG_HOME", xdg_config_dir);
        Self {
            home,
            xdg_config_home,
        }
    }
}

impl Drop for EnvContext {
    fn drop(&mut self) {
        if let Some(value) = &self.home {
            std::env::set_var("HOME", value);
        }
        if self.home.is_none() {
            std::env::remove_var("HOME");
        }
        if let Some(value) = &self.xdg_config_home {
            std::env::set_var("XDG_CONFIG_HOME", value);
        }
        if self.xdg_config_home.is_none() {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
    }
}

fn write_json(path: &Path, value: &Value) {
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn write_hotkeys(value: Value) {
    write_json(
        &paths::hotkeys_path().unwrap(),
        &json!({ "hotkeys": value }),
    );
}

fn write_shortcuts(value: Value) {
    write_json(
        &paths::shortcuts_path().unwrap(),
        &json!({ "shortcuts": value }),
    );
}

fn write_task_runner(value: Value) {
    write_json(&paths::task_runner_config_path().unwrap(), &value);
}

fn live_plugin_config_path(env: &TestEnv, plugin_id: &str) -> PathBuf {
    env.plugins_dir.join(plugin_id).join("config.json")
}

fn write_live_plugin_config(env: &TestEnv, plugin_id: &str, value: &Value) {
    write_json(&live_plugin_config_path(env, plugin_id), value);
}

fn read_live_plugin_config(env: &TestEnv, plugin_id: &str) -> Value {
    serde_json::from_slice(&fs::read(live_plugin_config_path(env, plugin_id)).unwrap()).unwrap()
}

fn build_export_bundle(env: &TestEnv) -> ProfileExportBundle {
    let _ctx = env.enter();
    profile::build_export_bundle("2026-03-29T00:00:00+00:00".to_string(), Vec::new()).unwrap()
}

fn build_export_bundle_json(env: &TestEnv) -> String {
    let _ctx = env.enter();
    profile::build_export_bundle_json("2026-03-29T00:00:00+00:00".to_string(), Vec::new()).unwrap()
}

fn import_bundle_from_export(bundle: &ProfileExportBundle) -> ProfileImportBundle {
    serde_json::from_value(serde_json::to_value(bundle).unwrap()).unwrap()
}

fn sync_request(remote_dir: &Path) -> SyncConnectRequest {
    SyncConnectRequest::Folder {
        folder_path: remote_dir.display().to_string(),
        path: "qol-tray/profile.json".to_string(),
        pull_on_launch: true,
        push_on_change: true,
    }
}

fn write_remote_profile(remote_dir: &Path, content: &str) {
    let path = remote_dir.join("qol-tray/profile.json");
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    fs::write(path, content).unwrap();
}

fn read_remote_profile(remote_dir: &Path) -> Value {
    serde_json::from_slice(&fs::read(remote_dir.join("qol-tray/profile.json")).unwrap()).unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn profile_export_import_round_trip_preserves_effective_state() {
    let _guard = env_lock().lock().await;
    let source = TestEnv::new();
    let target = TestEnv::new();

    {
        let _ctx = source.enter();
        profile::ensure_profile_dirs().unwrap();
        write_hotkeys(json!([{ "id": "hk-source" }]));
        write_shortcuts(json!([{ "id": "sc-source" }]));
        write_task_runner(json!({ "actions": { "source": {} } }));
        write_live_plugin_config(&source, "plugin-live", &json!({ "source": "live" }));
        write_live_plugin_config(&source, "plugin-shared", &json!({ "source": "live" }));
        profile::save_plugin_config("plugin-shared", &json!({ "source": "profile" })).unwrap();
    }

    {
        let _ctx = target.enter();
        profile::ensure_profile_dirs().unwrap();
        write_hotkeys(json!([{ "id": "hk-old" }]));
        write_shortcuts(json!([{ "id": "sc-old" }]));
        write_task_runner(json!({ "actions": { "old": {} } }));
        write_live_plugin_config(&target, "plugin-stale", &json!({ "stale": true }));
        fs::create_dir_all(target.plugins_dir.join("plugin-live")).unwrap();
        fs::create_dir_all(target.plugins_dir.join("plugin-shared")).unwrap();
    }

    let source_bundle = build_export_bundle(&source);
    let import_bundle = import_bundle_from_export(&source_bundle);

    {
        let _ctx = target.enter();
        profile::apply_import_bundle(&target.plugins_dir, &import_bundle)
            .await
            .unwrap();
        assert_eq!(
            profile::read_hotkeys_list(),
            vec![json!({ "id": "hk-source" })]
        );
        assert_eq!(
            profile::read_shortcuts_list(),
            vec![json!({ "id": "sc-source" })]
        );
        assert_eq!(
            profile::read_task_runner_value(),
            json!({ "actions": { "source": {} } })
        );
        assert_eq!(
            profile::load_plugin_config("plugin-live").unwrap(),
            Some(json!({ "source": "live" }))
        );
        assert_eq!(
            profile::load_plugin_config("plugin-shared").unwrap(),
            Some(json!({ "source": "profile" }))
        );
        assert_eq!(
            read_live_plugin_config(&target, "plugin-live"),
            json!({ "source": "live" })
        );
        assert_eq!(
            read_live_plugin_config(&target, "plugin-shared"),
            json!({ "source": "profile" })
        );
        assert!(!live_plugin_config_path(&target, "plugin-stale").exists());
    }

    let target_bundle = build_export_bundle(&target);

    assert_eq!(target_bundle.hotkeys, source_bundle.hotkeys);
    assert_eq!(target_bundle.shortcuts, source_bundle.shortcuts);
    assert_eq!(target_bundle.task_runner, source_bundle.task_runner);
    assert_eq!(target_bundle.plugin_configs, source_bundle.plugin_configs);
}

#[tokio::test(flavor = "current_thread")]
async fn folder_sync_connect_applies_remote_profile_and_creates_backup() {
    let _guard = env_lock().lock().await;
    let local = TestEnv::new();
    let remote_profile = TestEnv::new();
    let remote_dir = TempDir::new().unwrap();

    {
        let _ctx = local.enter();
        profile::ensure_profile_dirs().unwrap();
        write_hotkeys(json!([{ "id": "hk-local" }]));
        write_task_runner(json!({ "actions": { "local": {} } }));
    }

    {
        let _ctx = remote_profile.enter();
        profile::ensure_profile_dirs().unwrap();
        write_hotkeys(json!([{ "id": "hk-remote" }]));
        write_task_runner(json!({ "actions": { "remote": {} } }));
    }

    write_remote_profile(
        remote_dir.path(),
        &build_export_bundle_json(&remote_profile),
    );

    let service = {
        let _ctx = local.enter();
        SyncService::new(local.plugins_dir.clone()).unwrap()
    };

    let result = {
        let _ctx = local.enter();
        service
            .connect(sync_request(remote_dir.path()))
            .await
            .unwrap()
    };

    assert!(result.applied_remote);
    assert_eq!(result.status.health, SyncHealth::Attention);

    let backup_file = result
        .status
        .incident
        .as_ref()
        .and_then(|incident| incident.backup_file.clone())
        .unwrap();

    {
        let _ctx = local.enter();
        assert_eq!(
            profile::read_hotkeys_list(),
            vec![json!({ "id": "hk-remote" })]
        );
        assert_eq!(
            profile::read_task_runner_value(),
            json!({ "actions": { "remote": {} } })
        );
        assert_eq!(service.list_backups().unwrap().len(), 1);
        let preview = service.preview_backup(&backup_file).unwrap();
        let preview_json: Value = serde_json::from_str(&preview.content).unwrap();
        assert_eq!(
            preview_json["task_runner"],
            json!({ "actions": { "local": {} } })
        );
    }

    let ack_result = {
        let _ctx = local.enter();
        service.acknowledge_incident().await.unwrap()
    };

    assert_eq!(ack_result.status.health, SyncHealth::Healthy);
    assert!(ack_result.status.incident.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn folder_sync_push_conflict_backs_up_local_changes_and_applies_remote() {
    let _guard = env_lock().lock().await;
    let machine_a = TestEnv::new();
    let machine_b = TestEnv::new();
    let base = TestEnv::new();
    let remote_dir = TempDir::new().unwrap();

    {
        let _ctx = base.enter();
        profile::ensure_profile_dirs().unwrap();
        write_task_runner(json!({ "actions": { "base": {} } }));
    }

    {
        let _ctx = machine_a.enter();
        profile::ensure_profile_dirs().unwrap();
        write_task_runner(json!({ "actions": { "base": {} } }));
    }

    {
        let _ctx = machine_b.enter();
        profile::ensure_profile_dirs().unwrap();
        write_task_runner(json!({ "actions": { "base": {} } }));
    }

    write_remote_profile(remote_dir.path(), &build_export_bundle_json(&base));

    let service_a = {
        let _ctx = machine_a.enter();
        SyncService::new(machine_a.plugins_dir.clone()).unwrap()
    };
    let service_b = {
        let _ctx = machine_b.enter();
        SyncService::new(machine_b.plugins_dir.clone()).unwrap()
    };

    {
        let _ctx = machine_a.enter();
        service_a
            .connect(sync_request(remote_dir.path()))
            .await
            .unwrap();
    }

    {
        let _ctx = machine_b.enter();
        service_b
            .connect(sync_request(remote_dir.path()))
            .await
            .unwrap();
    }

    {
        let _ctx = machine_a.enter();
        write_task_runner(json!({ "actions": { "machine_a": {} } }));
        service_a.manual_push().await.unwrap();
    }

    {
        let _ctx = machine_b.enter();
        write_task_runner(json!({ "actions": { "machine_b": {} } }));
    }

    let result = {
        let _ctx = machine_b.enter();
        service_b.manual_push().await.unwrap()
    };

    assert!(result.applied_remote);
    assert_eq!(result.status.health, SyncHealth::Attention);

    let backup_file = result
        .status
        .incident
        .as_ref()
        .and_then(|incident| incident.backup_file.clone())
        .unwrap();

    {
        let _ctx = machine_b.enter();
        assert_eq!(
            profile::read_task_runner_value(),
            json!({ "actions": { "machine_a": {} } })
        );
        let preview = service_b.preview_backup(&backup_file).unwrap();
        let preview_json: Value = serde_json::from_str(&preview.content).unwrap();
        assert_eq!(
            preview_json["task_runner"],
            json!({ "actions": { "machine_b": {} } })
        );
    }

    assert_eq!(
        read_remote_profile(remote_dir.path())["task_runner"],
        json!({ "actions": { "machine_a": {} } })
    );

    let ack_result = {
        let _ctx = machine_b.enter();
        service_b.acknowledge_incident().await.unwrap()
    };
    assert_eq!(ack_result.status.health, SyncHealth::Healthy);
    assert!(ack_result.status.incident.is_none());

    let disconnect_result = {
        let _ctx = machine_b.enter();
        service_b.disconnect().await.unwrap()
    };
    assert_eq!(disconnect_result.status.health, SyncHealth::NotConfigured);
    assert!(!disconnect_result.status.configured);

    let reconnect_result = {
        let _ctx = machine_b.enter();
        service_b
            .connect(sync_request(remote_dir.path()))
            .await
            .unwrap()
    };
    assert!(reconnect_result.status.configured);
}
