#![cfg(unix)]
#![cfg(debug_assertions)]
#![cfg(feature = "dev")]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use qol_tray::plugins::action_executor;
use qol_tray::plugins::manager::PluginManager;
use qol_tray::plugins::registry::record_dev_link_create;
use tempfile::TempDir;
use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

const TEST_PATH_ROOT_ENV: &str = "QOL_TRAY_TEST_PATH_ROOT";

fn shared_root() -> &'static TempDir {
    static ROOT: OnceLock<TempDir> = OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = TempDir::new().expect("path-root tempdir");
        std::env::set_var(TEST_PATH_ROOT_ENV, dir.path());
        dir
    })
}

async fn lock_tests() -> AsyncMutexGuard<'static, ()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(())).lock().await
}

fn config_dir() -> PathBuf {
    shared_root().path().join("config").join("qol-tray")
}

fn plugins_dir() -> PathBuf {
    config_dir().join("plugins")
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod 755");
}

fn write_manifest(plugin_dir: &Path, socket_path: &Path) {
    let id = plugin_dir.file_name().unwrap().to_str().unwrap();
    let manifest = format!(
        r#"
[plugin]
id = "{id}"
name = "Action Dispatch Target"
description = "test"
version = "0.0.0"

[menu]
label = "Action"
items = []

[runtime]
command = "fake-runtime"
[runtime.actions]
do-thing = []

[daemon]
enabled = true
command = "fake-daemon"
socket = "{}"
"#,
        socket_path.display(),
    );
    fs::write(plugin_dir.join("plugin.toml"), manifest).expect("write plugin.toml");
}

fn install_plugin(plugin_id: &str, marker_path: &Path) -> (PathBuf, PathBuf) {
    let plugin_dir = plugins_dir().join(plugin_id);
    fs::create_dir_all(&plugin_dir).expect("plugin dir");

    let socket_path = shared_root().path().join(format!("{plugin_id}.sock"));

    write_executable(
        &plugin_dir.join("fake-daemon"),
        "#!/bin/sh\nexec sleep 60\n",
    );
    write_executable(
        &plugin_dir.join("fake-runtime"),
        &format!("#!/bin/sh\nprintf hello > {}\n", marker_path.display()),
    );
    write_manifest(&plugin_dir, &socket_path);

    record_dev_link_create(&config_dir(), plugin_id, plugin_dir.clone())
        .expect("dev-link the plugin");

    let effective_socket_path =
        qol_tray::dev_generation::daemon_socket_path(socket_path.to_str().unwrap());
    (plugin_dir, effective_socket_path)
}

struct FakeDaemon {
    handle: Option<JoinHandle<()>>,
    socket_path: PathBuf,
}

impl FakeDaemon {
    fn start(socket_path: &Path, response_line: &'static str) -> Self {
        let _ = fs::remove_file(socket_path);
        qol_fs::create_private_dir(socket_path.parent().unwrap())
            .expect("create fake daemon socket directory");
        let listener =
            qol_runtime::local_ipc::bind_listener(socket_path).expect("bind fake daemon socket");
        let handle = thread::Builder::new()
            .name("fake-daemon".into())
            .spawn(move || serve_fake_daemon(listener, response_line))
            .expect("spawn fake daemon thread");
        Self {
            handle: Some(handle),
            socket_path: socket_path.to_path_buf(),
        }
    }

    fn start_after(socket_path: &Path, response_line: &'static str, delay: Duration) -> Self {
        let socket_path = socket_path.to_path_buf();
        let thread_socket_path = socket_path.clone();
        let handle = thread::Builder::new()
            .name("delayed-fake-daemon".into())
            .spawn(move || {
                thread::sleep(delay);
                let _ = fs::remove_file(&thread_socket_path);
                qol_fs::create_private_dir(thread_socket_path.parent().unwrap())
                    .expect("create delayed fake daemon socket directory");
                let listener = qol_runtime::local_ipc::bind_listener(&thread_socket_path)
                    .expect("bind delayed fake daemon socket");
                serve_fake_daemon(listener, response_line);
            })
            .expect("spawn delayed fake daemon thread");
        Self {
            handle: Some(handle),
            socket_path,
        }
    }
}

fn serve_fake_daemon(listener: UnixListener, response_line: &str) {
    loop {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut reader = BufReader::new(stream.try_clone().expect("clone fake daemon stream"));
        let mut line = String::new();
        let Ok(_) = reader.read_line(&mut line) else {
            return;
        };
        if line.is_empty() {
            continue;
        }
        if line.trim() == "shutdown" {
            return;
        }

        if line.contains(r#""action":"ping""#) {
            let _ = stream.write_all(br#"{"status":"handled"}"#);
            let _ = stream.write_all(b"\n");
            continue;
        }

        let _ = stream.write_all(response_line.as_bytes());
        let _ = stream.write_all(b"\n");
        return;
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&self.socket_path) {
                let _ = stream.write_all(b"shutdown\n");
            }
            let _ = handle.join();
        }
    }
}

fn wait_until<F: FnMut() -> bool>(mut predicate: F, deadline: Duration) -> bool {
    let limit = Instant::now() + deadline;
    while Instant::now() < limit {
        if predicate() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    predicate()
}

fn loaded_manager() -> Arc<StdMutex<PluginManager>> {
    let mut manager = PluginManager::new();
    manager.load_plugins().expect("load_plugins after dev-link");
    Arc::new(StdMutex::new(manager))
}

#[tokio::test]
async fn action_dispatch_falls_back_to_runtime_when_daemon_replies_fallback() {
    let _serial = lock_tests().await;
    let plugin_id = "fallback-case";
    let marker = shared_root().path().join(format!("{plugin_id}.marker"));
    let _ = fs::remove_file(&marker);

    let (_plugin_dir, socket_path) = install_plugin(plugin_id, &marker);
    let _server = FakeDaemon::start(&socket_path, r#"{"status":"fallback"}"#);

    let manager = loaded_manager();
    action_executor::execute_action(&manager, plugin_id, "do-thing");

    assert!(
        wait_until(|| marker.exists(), Duration::from_secs(2)),
        "runtime fallback must run when daemon replies fallback (marker={})",
        marker.display(),
    );
    let body = fs::read_to_string(&marker).expect("read marker");
    assert_eq!(body, "hello", "runtime script must have written the marker");
}

#[tokio::test]
async fn action_dispatch_skips_runtime_when_daemon_replies_handled() {
    let _serial = lock_tests().await;
    let plugin_id = "handled-case";
    let marker = shared_root().path().join(format!("{plugin_id}.marker"));
    let _ = fs::remove_file(&marker);

    let (_plugin_dir, socket_path) = install_plugin(plugin_id, &marker);
    let _server = FakeDaemon::start(&socket_path, r#"{"status":"handled"}"#);

    let manager = loaded_manager();
    action_executor::execute_action(&manager, plugin_id, "do-thing");

    thread::sleep(Duration::from_millis(300));
    assert!(
        !marker.exists(),
        "runtime must NOT run when daemon replies handled (marker={})",
        marker.display(),
    );
}

#[tokio::test]
async fn action_dispatch_preserves_daemon_result_data() {
    let _serial = lock_tests().await;
    let plugin_id = "handled-data-case";
    let marker = shared_root().path().join(format!("{plugin_id}.marker"));
    let (_plugin_dir, socket_path) = install_plugin(plugin_id, &marker);
    let _server = FakeDaemon::start(
        &socket_path,
        r#"{"status":"handled","data":{"scheme":"dark","dark":true}}"#,
    );

    let manager = loaded_manager();
    let result = action_executor::try_execute_action_with_input_result(
        &manager,
        plugin_id,
        "do-thing",
        serde_json::Value::Null,
    )
    .unwrap();

    assert_eq!(
        result,
        Some(serde_json::json!({"scheme": "dark", "dark": true}))
    );
}

#[tokio::test]
async fn query_dispatch_recovers_when_daemon_socket_becomes_ready_after_request() {
    let _serial = lock_tests().await;
    let plugin_id = "delayed-query-case";
    let marker = shared_root().path().join(format!("{plugin_id}.marker"));
    let _ = fs::remove_file(&marker);

    let (_plugin_dir, socket_path) = install_plugin(plugin_id, &marker);
    let server = FakeDaemon::start_after(
        &socket_path,
        r#"{"status":"handled","data":{"items":[{"name":"controller"}]}}"#,
        Duration::from_millis(200),
    );
    let manager = loaded_manager();
    let started = Instant::now();

    let result = action_executor::dispatch_query(&manager, plugin_id, "controllers_snapshot");
    let elapsed = started.elapsed();
    drop(server);
    manager.lock().unwrap().shutdown();

    assert_eq!(
        result.unwrap(),
        serde_json::json!({"items": [{"name": "controller"}]}),
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "readiness probes must not inherit the 10-second action timeout"
    );
}
