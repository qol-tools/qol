use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::super::helpers::validate_plugin_id_bad_request;
use super::super::types::AppState;
use super::http_json::{self, blocking};
use crate::plugins::manager::reload_delivery::{
    CompletionDecision, PendingReloadTicket, ReloadDeliveryOutcome,
};
use crate::plugins::PluginManager;
use std::sync::{Arc, Mutex};

mod form;
mod io;
mod notify;

#[cfg(feature = "dev")]
pub(in super::super) use notify::notify_plugin_reload;

#[cfg(all(test, unix))]
mod save_phases {
    use std::cell::RefCell;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) enum SavePhase {
        BeforeSave,
        BeforeNotify,
    }

    type Hook = Box<dyn FnOnce() + Send>;

    thread_local! {
        static HOOKS: RefCell<Vec<(SavePhase, Hook)>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn set(phase: SavePhase, hook: impl FnOnce() + Send + 'static) {
        HOOKS.with(|hooks| hooks.borrow_mut().push((phase, Box::new(hook))));
    }

    pub(super) fn run(phase: SavePhase) {
        let hook = HOOKS.with(|hooks| {
            let mut hooks = hooks.borrow_mut();
            let index = hooks
                .iter()
                .position(|(registered, _)| *registered == phase)?;
            Some(hooks.remove(index).1)
        });
        if let Some(hook) = hook {
            hook();
        }
    }
}

#[cfg(all(test, unix))]
use save_phases::{run as run_save_phase_hook, SavePhase};

type HttpResult<T> = Result<T, Box<Response>>;

pub(in super::super) async fn get_plugin_config(
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    blocking("plugin config", move || get_plugin_config_inner(plugin_id)).await
}

pub(in super::super) async fn get_plugin_config_form(
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    blocking("plugin config", move || {
        get_plugin_config_form_inner(plugin_id)
    })
    .await
}

pub(in super::super) async fn set_plugin_config(
    Path(plugin_id): Path<String>,
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    blocking("plugin config", move || {
        set_plugin_config_inner(plugin_id, &state, body)
    })
    .await
}

fn get_plugin_config_inner(plugin_id: String) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let config = io::load_plugin_config(&plugin_id)?;
    Ok(config_json_response(&config))
}

fn get_plugin_config_form_inner(plugin_id: String) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let form = form::load_plugin_config_form(&plugin_id)?;
    Ok(config_form_json_response(&form))
}

fn set_plugin_config_inner(
    plugin_id: String,
    state: &AppState,
    body: axum::body::Bytes,
) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let config = io::parse_config_body(body)?;
    let config = form::normalize_plugin_config(&plugin_id, config)?;
    form::validate_plugin_config(&plugin_id, &config)?;
    apply_tracked_plugin_config_save(&plugin_id, config, &state.plugin_manager)
}

fn apply_tracked_plugin_config_save(
    plugin_id: &str,
    config: serde_json::Value,
    plugin_manager: &Arc<Mutex<PluginManager>>,
) -> HttpResult<Response> {
    let ticket = match begin_config_reload(plugin_manager, plugin_id) {
        Ok(ticket) => ticket,
        Err(error) => {
            log::error!("Config for {} was not saved: {}", plugin_id, error);
            return Err(Box::new(config_tracking_failed_response()));
        }
    };
    #[cfg(all(test, unix))]
    run_save_phase_hook(SavePhase::BeforeSave);
    let receipt = match io::save_plugin_config(plugin_id, config) {
        Ok(receipt) => receipt,
        Err(response) => {
            retire_config_reload(plugin_manager, &ticket, ReloadDeliveryOutcome::SaveFailed);
            return Err(response);
        }
    };
    if let Err(error) = record_config_reload_saved(plugin_manager, &ticket, receipt) {
        log::warn!(
            "Config saved for {}, but the tracked reload was superseded: {}",
            plugin_id,
            error
        );
        return Ok(config_saved_response());
    }
    #[cfg(all(test, unix))]
    run_save_phase_hook(SavePhase::BeforeNotify);
    dispatch_config_reload(plugin_manager, plugin_id, &ticket)
}

fn begin_config_reload(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
) -> Result<PendingReloadTicket, String> {
    plugin_manager
        .lock()
        .map_err(|error| format!("Plugin manager mutex poisoned: {error}"))?
        .begin_config_reload(plugin_id)
}

fn record_config_reload_saved(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    ticket: &PendingReloadTicket,
    receipt: crate::plugins::config::ConfigSaveReceipt,
) -> Result<(), String> {
    plugin_manager
        .lock()
        .map_err(|error| format!("Plugin manager mutex poisoned: {error}"))?
        .record_config_reload_saved(ticket, receipt)
}

fn retire_config_reload(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    ticket: &PendingReloadTicket,
    outcome: ReloadDeliveryOutcome,
) {
    let Ok(mut manager) = plugin_manager.lock() else {
        return;
    };
    manager.complete_config_reload(ticket, outcome);
}

fn complete_config_reload(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    ticket: &PendingReloadTicket,
    outcome: ReloadDeliveryOutcome,
) -> Result<CompletionDecision, String> {
    let mut manager = plugin_manager
        .lock()
        .map_err(|error| format!("Plugin manager mutex poisoned: {error}"))?;
    Ok(manager.complete_config_reload(ticket, outcome))
}

fn dispatch_config_reload(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    ticket: &PendingReloadTicket,
) -> HttpResult<Response> {
    let outcome = match notify::notify_plugin_reload_outcome(plugin_manager, plugin_id) {
        Ok(outcome) => outcome,
        Err(error) => {
            retire_config_reload(
                plugin_manager,
                ticket,
                ReloadDeliveryOutcome::SnapshotFailed,
            );
            log::error!(
                "Config saved for {}, but daemon refresh failed: {}",
                plugin_id,
                error
            );
            return Err(Box::new(config_refresh_failed_response()));
        }
    };
    let decision = match complete_config_reload(plugin_manager, ticket, outcome) {
        Ok(decision) => decision,
        Err(error) => {
            log::error!(
                "Config saved for {}, but daemon refresh failed: {}",
                plugin_id,
                error
            );
            return Err(Box::new(config_refresh_failed_response()));
        }
    };
    if let CompletionDecision::RestartFailed(error) = decision {
        log::error!(
            "Config saved for {}, but daemon refresh failed: {}",
            plugin_id,
            error
        );
        return Err(Box::new(config_refresh_failed_response()));
    }
    Ok(config_saved_response())
}

fn config_tracking_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Config not saved: live daemon refresh could not be tracked",
    )
        .into_response()
}

fn validated_plugin_id(plugin_id: String) -> HttpResult<String> {
    validate_plugin_id_bad_request(&plugin_id).map_err(|e| Box::new(e.into_response()))?;
    Ok(plugin_id)
}

fn config_json_response(config: &serde_json::Value) -> Response {
    let json = match io::encode_config_json(config) {
        Ok(json) => json,
        Err(_) => return serialize_config_failed_response(),
    };
    http_json::json_response(json)
}

fn config_form_json_response(combined: &form::CombinedPluginForm) -> Response {
    let json = match http_json::encode_json(combined, "Failed to serialize config form") {
        Ok(json) => json,
        Err(_) => return serialize_config_form_failed_response(),
    };
    http_json::json_response(json)
}

fn config_saved_response() -> Response {
    (StatusCode::OK, "Config saved").into_response()
}

fn config_refresh_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Config saved but live daemon refresh failed",
    )
        .into_response()
}

fn serialize_config_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to serialize config",
    )
        .into_response()
}

fn serialize_config_form_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to serialize config form",
    )
        .into_response()
}

#[cfg(all(test, unix))]
mod tests {
    use super::save_phases::{set as set_save_phase_hook, SavePhase};
    use super::*;
    use crate::plugins::{PluginLoader, PluginManager};
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    const PLUGIN_ID: &str = "plugin-config-reload-handler";
    const SOCKET_NAME: &str = "config-reload-handler.sock";
    const BARRIER_TIMEOUT: Duration = Duration::from_secs(10);
    const WITNESS_TIMEOUT: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(10);

    struct Harness {
        manager: Arc<Mutex<PluginManager>>,
        config_path: PathBuf,
        socket_path: PathBuf,
        stopped: bool,
    }

    impl Harness {
        fn pid(&self) -> u32 {
            self.manager
                .lock()
                .unwrap()
                .get(PLUGIN_ID)
                .unwrap()
                .daemon_pid()
                .unwrap()
        }

        fn finish(mut self) {
            self.stop();
        }

        fn stop(&mut self) {
            if self.stopped {
                return;
            }
            self.stopped = true;
            if let Ok(mut manager) = self.manager.lock() {
                manager.shutdown();
            }
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn write_fixture_plugin(
        plugins_dir: &Path,
        witness_dir: &Path,
        contract_socket: &Path,
    ) -> PathBuf {
        let plugin_dir = plugins_dir.join(PLUGIN_ID);
        fs::create_dir_all(&plugin_dir).unwrap();
        let script = plugin_dir.join("daemon");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ -f \"$QOL_TRAY_PLUGIN_DIR/config.json\" ]; then cat \"$QOL_TRAY_PLUGIN_DIR/config.json\" > \"{}/applied.$$.json\"; fi\nexec sleep 30\n",
                witness_dir.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let contract_socket = contract_socket.display();
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                r#"
[plugin]
id = "{PLUGIN_ID}"
name = "{PLUGIN_ID}"
description = ""
version = "1.0.0"

[menu]
label = "{PLUGIN_ID}"
items = []

[daemon]
enabled = true
command = "daemon"
socket = "{contract_socket}"
"#
            ),
        )
        .unwrap();
        plugin_dir
    }

    fn harness(root: &tempfile::TempDir) -> Harness {
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let contract_socket = root.path().join(SOCKET_NAME);
        let plugin_dir = write_fixture_plugin(&plugins_dir, root.path(), &contract_socket);
        let config_dir = crate::paths::shared_config_dir().unwrap();
        crate::plugins::registry::record_release_install(&config_dir, PLUGIN_ID, plugin_dir)
            .unwrap();
        let mut manager = PluginManager::new();
        let plugin =
            PluginLoader::load_plugin_with_id(PLUGIN_ID, &plugins_dir.join(PLUGIN_ID)).unwrap();
        manager.insert_plugin_for_test(plugin);
        manager.ensure_plugin_daemon_running(PLUGIN_ID).unwrap();
        let socket_path = crate::dev_generation::daemon_socket_path(
            contract_socket
                .to_str()
                .expect("test contract socket must be valid UTF-8"),
        );
        Harness {
            manager: Arc::new(Mutex::new(manager)),
            config_path: plugins_dir.join(PLUGIN_ID).join("config.json"),
            socket_path,
            stopped: false,
        }
    }

    fn daemon_witness(witness_dir: &Path, pid: u32) -> PathBuf {
        witness_dir.join(format!("applied.{pid}.json"))
    }

    fn worker_apply(
        root: &Path,
        manager: &Arc<Mutex<PluginManager>>,
        value: u64,
    ) -> std::thread::JoinHandle<HttpResult<Response>> {
        let manager = Arc::clone(manager);
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            let _path = crate::paths::push_test_path_root(&root);
            apply_tracked_plugin_config_save(
                PLUGIN_ID,
                serde_json::json!({"value": value}),
                &manager,
            )
        })
    }

    fn wait_applied_value(path: &Path, expected: &serde_json::Value) {
        let deadline = Instant::now() + WITNESS_TIMEOUT;
        loop {
            let observation = match fs::read(path) {
                Ok(bytes) => {
                    if serde_json::from_slice::<serde_json::Value>(&bytes)
                        .is_ok_and(|value| value == *expected)
                    {
                        return;
                    }
                    String::from_utf8_lossy(&bytes).to_string()
                }
                Err(error) => format!("read error: {error}"),
            };
            assert!(
                Instant::now() < deadline,
                "applied witness {path:?} never reached {expected}; last observation: {observation:?}"
            );
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    enum ScriptAction {
        HandledNow,
        DropNow,
        DropAfter(mpsc::Receiver<()>),
    }

    struct ScriptedDaemon {
        arrived: mpsc::Receiver<usize>,
        path: PathBuf,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl ScriptedDaemon {
        fn start(
            path: &Path,
            actions: Vec<ScriptAction>,
            witness: Option<PathBuf>,
            config: PathBuf,
        ) -> Self {
            if let Some(parent) = path.parent() {
                qol_fs::create_private_dir(parent).unwrap();
            }
            let _ = fs::remove_file(path);
            let listener = UnixListener::bind(path).unwrap();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = Arc::clone(&stop);
            let (arrived_tx, arrived_rx) = mpsc::channel();
            let handle = std::thread::spawn(move || {
                for (index, action) in actions.into_iter().enumerate() {
                    let Some(stream) = accept_stream(&listener, &thread_stop) else {
                        return;
                    };
                    let _ = arrived_tx.send(index);
                    match action {
                        ScriptAction::HandledNow => {
                            respond_handled(stream, witness.as_deref(), &config)
                        }
                        ScriptAction::DropNow => drop(stream),
                        ScriptAction::DropAfter(release) => {
                            wait_for_release(&release, &thread_stop);
                            drop(stream);
                        }
                    }
                }
            });
            Self {
                arrived: arrived_rx,
                path: path.to_path_buf(),
                stop,
                handle: Some(handle),
            }
        }

        fn wait_arrival(&self, index: usize) {
            let arrived = self
                .arrived
                .recv_timeout(BARRIER_TIMEOUT)
                .unwrap_or_else(|_| panic!("scripted daemon connection {index} did not arrive"));
            assert_eq!(arrived, index, "daemon connection order");
        }

        fn join(mut self) {
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    impl Drop for ScriptedDaemon {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
            let _ = fs::remove_file(&self.path);
        }
    }

    fn accept_stream(listener: &UnixListener, stop: &AtomicBool) -> Option<UnixStream> {
        let deadline = Instant::now() + BARRIER_TIMEOUT;
        loop {
            if stop.load(Ordering::SeqCst) {
                return None;
            }
            match listener.accept() {
                Ok((stream, _)) => return Some(stream),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(_) => return None,
            }
        }
    }

    fn wait_for_release(release: &mpsc::Receiver<()>, stop: &AtomicBool) {
        let deadline = Instant::now() + BARRIER_TIMEOUT;
        while Instant::now() < deadline {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            match release.try_recv() {
                Ok(()) | Err(mpsc::TryRecvError::Disconnected) => return,
                Err(mpsc::TryRecvError::Empty) => std::thread::sleep(POLL_INTERVAL),
            }
        }
    }

    fn respond_handled(stream: UnixStream, witness: Option<&Path>, config: &Path) {
        let _ = stream.set_read_timeout(Some(BARRIER_TIMEOUT));
        let _ = stream.set_write_timeout(Some(BARRIER_TIMEOUT));
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        let _ = reader.read_line(&mut request);
        if let Some(witness) = witness {
            if let Ok(bytes) = fs::read(config) {
                let _ = fs::write(witness, bytes);
            }
        }
        let mut stream = stream;
        let _ = stream.write_all(b"{\"status\":\"handled\"}\n");
        let _ = stream.flush();
    }

    #[test]
    fn published_save_defers_reconciliation_until_the_daemon_accepts() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let harness = harness(&root);
        let pid_before = harness.pid();
        let witness = daemon_witness(root.path(), pid_before);

        let daemon = ScriptedDaemon::start(
            &harness.socket_path,
            vec![ScriptAction::HandledNow],
            Some(witness.clone()),
            harness.config_path.clone(),
        );

        let (published_tx, published_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let manager = Arc::clone(&harness.manager);
        let root_path = root.path().to_path_buf();
        let worker = std::thread::spawn(move || {
            let _path = crate::paths::push_test_path_root(&root_path);
            set_save_phase_hook(SavePhase::BeforeNotify, move || {
                let _ = published_tx.send(());
                let _ = release_rx.recv_timeout(BARRIER_TIMEOUT);
            });
            apply_tracked_plugin_config_save(PLUGIN_ID, serde_json::json!({"value": 1}), &manager)
        });

        published_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(harness
            .manager
            .lock()
            .unwrap()
            .has_pending_config_reload(PLUGIN_ID));
        assert!(!harness
            .manager
            .lock()
            .unwrap()
            .reconcile_profile_generation()
            .unwrap());
        assert_eq!(harness.pid(), pid_before);

        release_tx.send(()).unwrap();
        let response = worker.join().unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        daemon.wait_arrival(0);
        daemon.join();

        wait_applied_value(&witness, &serde_json::json!({"value": 1}));
        assert!(!harness
            .manager
            .lock()
            .unwrap()
            .reconcile_profile_generation()
            .unwrap());
        assert_eq!(harness.pid(), pid_before);
        harness.finish();
    }

    #[test]
    fn unreachable_notification_restarts_and_applies_the_saved_value() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let harness = harness(&root);
        let pid_before = harness.pid();

        let response = apply_tracked_plugin_config_save(
            PLUGIN_ID,
            serde_json::json!({"value": 7}),
            &harness.manager,
        )
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let pid_after = harness.pid();
        assert_ne!(pid_after, pid_before);
        wait_applied_value(
            &daemon_witness(root.path(), pid_after),
            &serde_json::json!({"value": 7}),
        );
        assert!(!harness
            .manager
            .lock()
            .unwrap()
            .reconcile_profile_generation()
            .unwrap());
        assert_eq!(harness.pid(), pid_after);
        harness.finish();
    }

    #[test]
    fn replacement_during_notification_failure_is_not_restarted() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let harness = harness(&root);

        let (release_tx, release_rx) = mpsc::channel();
        let daemon = ScriptedDaemon::start(
            &harness.socket_path,
            vec![ScriptAction::DropAfter(release_rx)],
            None,
            harness.config_path.clone(),
        );

        let worker = worker_apply(root.path(), &harness.manager, 1);
        daemon.wait_arrival(0);

        let replacement_pid = {
            let mut manager = harness.manager.lock().unwrap();
            manager.restart_running_plugin_daemon(PLUGIN_ID).unwrap();
            manager.get(PLUGIN_ID).unwrap().daemon_pid().unwrap()
        };
        release_tx.send(()).unwrap();

        let response = worker.join().unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        daemon.join();
        assert_eq!(harness.pid(), replacement_pid);
        assert!(!harness
            .manager
            .lock()
            .unwrap()
            .reconcile_profile_generation()
            .unwrap());
        assert_eq!(harness.pid(), replacement_pid);
        harness.finish();
    }

    #[test]
    fn newer_accepted_save_prevents_old_failure_restart() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let harness = harness(&root);

        let daemon = ScriptedDaemon::start(
            &harness.socket_path,
            vec![ScriptAction::HandledNow, ScriptAction::DropNow],
            None,
            harness.config_path.clone(),
        );

        let (published_tx, published_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let manager = Arc::clone(&harness.manager);
        let root_path = root.path().to_path_buf();
        let older = std::thread::spawn(move || {
            let _path = crate::paths::push_test_path_root(&root_path);
            set_save_phase_hook(SavePhase::BeforeNotify, move || {
                let _ = published_tx.send(());
                let _ = release_rx.recv_timeout(BARRIER_TIMEOUT);
            });
            apply_tracked_plugin_config_save(PLUGIN_ID, serde_json::json!({"value": 1}), &manager)
        });
        published_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let pid_before = harness.pid();

        let newer = apply_tracked_plugin_config_save(
            PLUGIN_ID,
            serde_json::json!({"value": 2}),
            &harness.manager,
        )
        .unwrap();
        assert_eq!(newer.status(), StatusCode::OK);
        daemon.wait_arrival(0);

        release_tx.send(()).unwrap();
        let older_response = older.join().unwrap().unwrap();
        assert_eq!(older_response.status(), StatusCode::OK);
        daemon.wait_arrival(1);
        daemon.join();

        assert_eq!(harness.pid(), pid_before);
        assert!(!harness
            .manager
            .lock()
            .unwrap()
            .reconcile_profile_generation()
            .unwrap());
        assert_eq!(harness.pid(), pid_before);
        harness.finish();
    }

    #[test]
    fn failing_first_published_request_does_not_restart_while_smaller_request_pending() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let harness = harness(&root);

        let daemon = ScriptedDaemon::start(
            &harness.socket_path,
            vec![ScriptAction::DropNow, ScriptAction::HandledNow],
            None,
            harness.config_path.clone(),
        );

        let (registered_tx, registered_rx) = mpsc::channel();
        let (save_release_tx, save_release_rx) = mpsc::channel();
        let manager = Arc::clone(&harness.manager);
        let root_path = root.path().to_path_buf();
        let smaller = std::thread::spawn(move || {
            let _path = crate::paths::push_test_path_root(&root_path);
            set_save_phase_hook(SavePhase::BeforeSave, move || {
                let _ = registered_tx.send(());
                let _ = save_release_rx.recv_timeout(BARRIER_TIMEOUT);
            });
            apply_tracked_plugin_config_save(PLUGIN_ID, serde_json::json!({"value": 2}), &manager)
        });
        registered_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let (published_tx, published_rx) = mpsc::channel();
        let (notify_release_tx, notify_release_rx) = mpsc::channel();
        let manager = Arc::clone(&harness.manager);
        let root_path = root.path().to_path_buf();
        let larger = std::thread::spawn(move || {
            let _path = crate::paths::push_test_path_root(&root_path);
            set_save_phase_hook(SavePhase::BeforeNotify, move || {
                let _ = published_tx.send(());
                let _ = notify_release_rx.recv_timeout(BARRIER_TIMEOUT);
            });
            apply_tracked_plugin_config_save(PLUGIN_ID, serde_json::json!({"value": 1}), &manager)
        });
        published_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let pid_before = harness.pid();

        notify_release_tx.send(()).unwrap();
        let larger_response = larger.join().unwrap().unwrap();
        assert_eq!(larger_response.status(), StatusCode::OK);
        daemon.wait_arrival(0);
        assert_eq!(harness.pid(), pid_before);

        save_release_tx.send(()).unwrap();
        let smaller_response = smaller.join().unwrap().unwrap();
        assert_eq!(smaller_response.status(), StatusCode::OK);
        daemon.wait_arrival(1);
        daemon.join();

        assert_eq!(harness.pid(), pid_before);
        assert!(!harness
            .manager
            .lock()
            .unwrap()
            .reconcile_profile_generation()
            .unwrap());
        assert_eq!(harness.pid(), pid_before);
        harness.finish();
    }

    #[test]
    fn enrollment_failure_fails_before_publishing() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let manager = Arc::new(Mutex::new(PluginManager::new()));

        let response = apply_tracked_plugin_config_save(
            "plugin-not-enrolled",
            serde_json::json!({"value": 1}),
            &manager,
        );
        assert_eq!(
            response.unwrap_err().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            crate::plugins::config::profile_config_plugin_generation("plugin-not-enrolled"),
            0
        );
        assert!(
            !crate::paths::plugins_dir()
                .unwrap()
                .join("plugin-not-enrolled")
                .join("config.json")
                .exists(),
            "a registration failure must not publish config bytes"
        );
    }

    #[test]
    fn no_daemon_save_is_valid_and_tracked() {
        const NO_DAEMON_ID: &str = "plugin-config-reload-no-daemon";

        let _env = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let plugin_dir = plugins_dir.join(NO_DAEMON_ID);
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                r#"
[plugin]
id = "{NO_DAEMON_ID}"
name = "{NO_DAEMON_ID}"
description = ""
version = "1.0.0"

[menu]
label = "{NO_DAEMON_ID}"
items = []
"#
            ),
        )
        .unwrap();
        let config_dir = crate::paths::shared_config_dir().unwrap();
        crate::plugins::registry::record_release_install(
            &config_dir,
            NO_DAEMON_ID,
            plugin_dir.clone(),
        )
        .unwrap();
        let mut manager = PluginManager::new();
        let plugin = PluginLoader::load_plugin_with_id(NO_DAEMON_ID, &plugin_dir).unwrap();
        manager.insert_plugin_for_test(plugin);
        let manager = Arc::new(Mutex::new(manager));

        let response = apply_tracked_plugin_config_save(
            NO_DAEMON_ID,
            serde_json::json!({"value": 1}),
            &manager,
        )
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!manager
            .lock()
            .unwrap()
            .has_pending_config_reload(NO_DAEMON_ID));
        assert!(plugin_dir.join("config.json").exists());
        manager.lock().unwrap().shutdown();
    }

    #[test]
    fn failing_save_retires_its_request_and_reconcile_still_applies() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let _runtime_cache = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let harness = harness(&root);

        let seed = crate::plugins::PluginConfigManager::new()
            .unwrap()
            .set_config_tracked(PLUGIN_ID, serde_json::json!({"value": 0}))
            .unwrap();
        harness
            .manager
            .lock()
            .unwrap()
            .ensure_plugin_daemon_running(PLUGIN_ID)
            .unwrap();
        let pid_before = harness.pid();

        {
            let _cold_cache = crate::plugins::config::begin_runtime_config_global_mutation();
        }
        fs::remove_file(&harness.config_path).unwrap();
        fs::create_dir(&harness.config_path).unwrap();
        let early = apply_tracked_plugin_config_save(
            PLUGIN_ID,
            serde_json::json!({"value": 1}),
            &harness.manager,
        );
        assert_eq!(
            early.unwrap_err().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert!(!harness
            .manager
            .lock()
            .unwrap()
            .has_pending_config_reload(PLUGIN_ID));
        assert_eq!(
            crate::plugins::config::profile_config_plugin_generation(PLUGIN_ID),
            seed.generation,
            "an early persistence failure must not publish an invalidation"
        );
        assert_eq!(harness.pid(), pid_before);

        fs::remove_dir(&harness.config_path).unwrap();
        let retained = crate::plugins::PluginConfigManager::new()
            .unwrap()
            .get_config(PLUGIN_ID)
            .unwrap();
        assert_eq!(retained, Some(serde_json::json!({"value": 0})));

        fs::remove_file(&harness.config_path).unwrap();
        fs::create_dir(&harness.config_path).unwrap();
        let published = apply_tracked_plugin_config_save(
            PLUGIN_ID,
            serde_json::json!({"value": 2}),
            &harness.manager,
        );
        assert_eq!(
            published.unwrap_err().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert!(!harness
            .manager
            .lock()
            .unwrap()
            .has_pending_config_reload(PLUGIN_ID));
        assert!(
            crate::plugins::config::profile_config_plugin_generation(PLUGIN_ID) > seed.generation,
            "a post-publication persistence failure must keep its invalidation"
        );
        assert_eq!(harness.pid(), pid_before);

        fs::remove_dir(&harness.config_path).unwrap();
        assert!(harness
            .manager
            .lock()
            .unwrap()
            .reconcile_profile_generation()
            .unwrap());
        let pid_after = harness.pid();
        assert_ne!(pid_after, pid_before);
        wait_applied_value(
            &daemon_witness(root.path(), pid_after),
            &serde_json::json!({"value": 0}),
        );
        assert!(!harness
            .manager
            .lock()
            .unwrap()
            .reconcile_profile_generation()
            .unwrap());
        assert_eq!(harness.pid(), pid_after);
        harness.finish();
    }
}
