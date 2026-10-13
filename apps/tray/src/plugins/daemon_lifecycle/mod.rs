pub mod lifeline_handoff;
mod listener;
mod platform;
mod spawn;

use super::Plugin;
use anyhow::Result;
use std::collections::HashMap;
use std::process::Child;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub(super) use listener::DaemonListener;

const DAEMON_STOP_GRACE: Duration = Duration::from_secs(3);
const SESSION_END_POLL: Duration = Duration::from_millis(20);

#[derive(Default)]
struct Incarnation {
    generation: u64,
    token: Option<String>,
    endpoint: Option<std::path::PathBuf>,
    artifact: Option<qol_artifact::InspectedArtifact>,
    process: Option<DaemonProcess>,
}

#[derive(Clone)]
struct DaemonProcess {
    pid: u32,
    identity: String,
}

pub(super) struct StoppingDaemon {
    plugin_id: String,
    child: Child,
}

static DAEMON_INCARNATIONS: OnceLock<Mutex<HashMap<String, Incarnation>>> = OnceLock::new();

fn daemon_incarnations() -> &'static Mutex<HashMap<String, Incarnation>> {
    DAEMON_INCARNATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn bump_daemon_incarnation(plugin_id: &str) -> u64 {
    let mut incarnations = daemon_incarnations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = incarnations.entry(plugin_id.to_string()).or_default();
    entry.generation = entry.generation.saturating_add(1);
    entry.token = None;
    entry.endpoint = None;
    entry.artifact = None;
    entry.process = None;
    entry.generation
}

pub(super) fn current_daemon_incarnation(plugin_id: &str) -> Option<u64> {
    daemon_incarnations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(plugin_id)
        .map(|entry| entry.generation)
}

pub(super) fn start_daemon(plugin: &mut Plugin) -> Result<()> {
    start_daemon_with_context(plugin, None).map(|_| ())
}

pub(super) fn start_daemon_with_context(
    plugin: &mut Plugin,
    runtime_config: Option<&mut crate::plugins::config::RuntimeConfigContext>,
) -> Result<Option<u64>> {
    let Some(daemon_config) = spawn::enabled_daemon(plugin).cloned() else {
        return Ok(None);
    };
    if crate::features::linked_devices::is_legacy_pointz(&plugin.manifest)
        && !crate::features::linked_devices::legacy_pointz_allowed()
    {
        anyhow::bail!(
            "{} must be updated: phone pairing now belongs to linked devices",
            plugin.manifest.plugin.name
        );
    }

    reap_daemon_if_exited(plugin);
    if let Some(pid) = plugin.daemon_pid() {
        log::info!(
            "Daemon for plugin {} is already running as pid {}; not spawning a second one",
            plugin.id,
            pid
        );
        return Ok(None);
    }

    if let Some(existing) = plugin.daemon_listener.take() {
        plugin.daemon_listener = listener::refresh_for_respawn(existing, plugin, &daemon_config);
    }
    if plugin.daemon_listener.is_none() {
        plugin.daemon_listener = listener::bind_for_plugin(plugin, &daemon_config);
    }

    let token = fresh_instance_token()?;
    let (child, consumed_generation, spawn_fingerprint, artifact) = spawn::spawn_daemon(
        plugin,
        &daemon_config,
        plugin.daemon_listener.as_ref(),
        runtime_config,
        &token,
    )?;
    register_daemon(plugin, child, spawn_fingerprint);
    let mut incarnations = daemon_incarnations()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let incarnation = incarnations.entry(plugin.id.to_string()).or_default();
    incarnation.token = Some(token);
    incarnation.artifact = artifact;
    Ok(Some(consumed_generation))
}

pub(super) fn existing_daemon_socket_ready(plugin: &Plugin) -> bool {
    // When qol-tray holds the pre-bound listener itself, connecting to the
    // socket succeeds regardless of daemon state - the probe would only be
    // measuring our own fd. Any daemon on that socket was spawned by this
    // process, so its child liveness is the real signal.
    if plugin.daemon_listener.is_some() {
        return plugin
            .daemon_pid()
            .is_some_and(|pid| crate::process_utils::is_pid_alive(pid as i32));
    }

    let Some(socket_path) = plugin
        .manifest
        .daemon
        .as_ref()
        .filter(|daemon| daemon.enabled)
        .and_then(|daemon| daemon.socket.as_deref())
        .map(crate::dev_generation::daemon_socket_path)
    else {
        return false;
    };

    super::action_transport::daemon_listener_reachable(&socket_path)
}

pub(super) fn reap_daemon_if_exited(plugin: &mut Plugin) {
    let Some(child) = plugin.daemon_process.as_mut() else {
        return;
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            clear_exited_daemon(plugin, &format!("exited unexpectedly ({})", status))
        }
        Ok(None) => {}
        Err(error) if reaped_elsewhere(&error) => {
            clear_exited_daemon(plugin, "was already reaped elsewhere")
        }
        Err(error) => {
            log::warn!(
                "Failed to poll daemon status for plugin {}: {}",
                plugin.id,
                error
            );
        }
    }
}

fn clear_exited_daemon(plugin: &mut Plugin, reason: &str) {
    let Some(child) = &plugin.daemon_process else {
        return;
    };
    let pid = child.id();
    log::warn!(
        "Daemon for plugin {} {}, clearing pid {}",
        plugin.id,
        reason,
        pid
    );
    super::daemon_tracker::registry::unregister(
        &crate::paths::runtime_pids_dir(),
        plugin.id.as_str(),
        pid,
    );
    plugin.daemon_process = None;
    plugin.daemon_spawn_fingerprint = None;
    bump_daemon_incarnation(plugin.id.as_str());
}

fn reaped_elsewhere(error: &std::io::Error) -> bool {
    platform::reaped_elsewhere(error)
}

pub(super) fn stop_daemon(plugin: &mut Plugin) -> Result<()> {
    finish_stops(begin_stop(plugin).into_iter().collect());
    Ok(())
}

pub(super) fn begin_stop(plugin: &mut Plugin) -> Option<StoppingDaemon> {
    // qol-tray keeps its own copy of a pre-bound listener open so a crashed
    // daemon can respawn onto the same fd without re-binding (see
    // listener::bind_for_plugin). An explicit stop has no respawn coming, so
    // that copy must close too - otherwise the socket stays "reachable" with
    // nothing behind it to ever accept a connection, and callers hang for a
    // full timeout instead of getting refused immediately. Dropped before the
    // no-child early return: the daemon may already have crashed and been
    // reaped by the time the stop arrives.
    plugin.daemon_listener = None;

    let child = plugin.daemon_process.take()?;
    plugin.daemon_spawn_fingerprint = None;
    bump_daemon_incarnation(plugin.id.as_str());

    log::info!("Stopping daemon for plugin: {}", plugin.id);
    super::daemon_tracker::registry::unregister(
        &crate::paths::runtime_pids_dir(),
        plugin.id.as_str(),
        child.id(),
    );
    Some(StoppingDaemon {
        plugin_id: plugin.id.to_string(),
        child,
    })
}

pub(super) fn finish_stops(daemons: Vec<StoppingDaemon>) {
    finish_stops_by(daemons, Instant::now() + DAEMON_STOP_GRACE);
}

fn finish_stops_by(daemons: Vec<StoppingDaemon>, deadline: Instant) {
    std::thread::scope(|scope| {
        for mut daemon in daemons {
            scope.spawn(move || finish_stop(&mut daemon, deadline));
        }
    });
}

fn finish_stop(daemon: &mut StoppingDaemon, deadline: Instant) {
    let grace = deadline.saturating_duration_since(Instant::now());
    if let Err(error) = crate::process_utils::terminate_owned(&mut daemon.child, grace) {
        log::warn!("Error reaping daemon for {}: {}", daemon.plugin_id, error);
    }
}

pub(crate) fn stop_daemons_for_session_end(deadline: Instant) {
    let signaled = live_daemon_processes()
        .into_iter()
        .filter(|(plugin_id, process)| request_session_end_stop(plugin_id, process))
        .map(|(_, process)| process.pid)
        .collect();
    let running = wait_until_gone(signaled, deadline, qol_process::is_pid_gone);
    if !running.is_empty() {
        log::warn!("Daemons still running when the session ends: {running:?}");
    }
}

fn live_daemon_processes() -> Vec<(String, DaemonProcess)> {
    daemon_incarnations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter_map(|(plugin_id, entry)| Some((plugin_id.clone(), entry.process.clone()?)))
        .collect()
}

fn request_session_end_stop(plugin_id: &str, process: &DaemonProcess) -> bool {
    match qol_process::request_stop_if_identity(process.pid, &process.identity) {
        Ok(requested) => requested,
        Err(error) => {
            log::warn!(
                "Failed to stop daemon {} for plugin {plugin_id} at session end: {error}",
                process.pid
            );
            false
        }
    }
}

fn wait_until_gone(mut pids: Vec<u32>, deadline: Instant, gone: impl Fn(u32) -> bool) -> Vec<u32> {
    loop {
        pids.retain(|pid| !gone(*pid));
        let now = Instant::now();
        if pids.is_empty() || now >= deadline {
            return pids;
        }
        std::thread::sleep(SESSION_END_POLL.min(deadline - now));
    }
}

fn register_daemon(plugin: &mut Plugin, child: Child, spawn_fingerprint: Option<String>) {
    let pid = child.id();
    if let Some(mut previous) = plugin.daemon_process.take() {
        log::warn!(
            "Replacing a live daemon for plugin {}: killing orphaned pid {}",
            plugin.id,
            previous.id()
        );
        let _ = previous.kill();
        let _ = previous.wait();
    }
    plugin.daemon_process = Some(child);
    plugin.daemon_spawn_fingerprint = spawn_fingerprint;
    bump_daemon_incarnation(plugin.id.as_str());
    let identity = qol_process::process_identity(pid).ok();
    {
        let mut incarnations = daemon_incarnations()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let incarnation = incarnations.entry(plugin.id.to_string()).or_default();
        incarnation.endpoint = super::action_executor::daemon_socket(plugin);
        incarnation.process = identity.map(|identity| DaemonProcess { pid, identity });
    }
    if let Err(error) = qol_process::bind_to_host_lifetime(pid) {
        log::warn!(
            "plugin {} daemon {pid} is not bound to the tray lifetime: {error}",
            plugin.id
        );
    }
    crate::desktop_state::add_ignore_pid(pid);
    super::daemon_tracker::registry::register(
        &crate::paths::runtime_pids_dir(),
        plugin.id.as_str(),
        pid,
    );
    log::info!("Registered ignore pid {} for plugin {}", pid, plugin.id);
}

pub(crate) fn current_daemon_token(plugin_id: &str) -> Option<String> {
    daemon_incarnations()
        .lock()
        .ok()?
        .get(plugin_id)?
        .token
        .clone()
}

fn fresh_instance_token() -> Result<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("daemon instance randomness unavailable"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) fn register_operation_fixture(
    plugin: &mut Plugin,
    child: Child,
    token: String,
    artifact: qol_artifact::InspectedArtifact,
) {
    register_daemon(plugin, child, None);
    let mut incarnations = daemon_incarnations().lock().unwrap();
    let incarnation = incarnations.entry(plugin.id.to_string()).or_default();
    incarnation.token = Some(token);
    incarnation.artifact = Some(artifact);
}

pub(crate) fn current_daemon_endpoint(plugin_id: &str) -> Option<std::path::PathBuf> {
    daemon_incarnations()
        .lock()
        .ok()?
        .get(plugin_id)?
        .endpoint
        .clone()
}

pub(crate) fn current_daemon_artifact(plugin_id: &str) -> Option<qol_artifact::InspectedArtifact> {
    daemon_incarnations()
        .lock()
        .ok()?
        .get(plugin_id)?
        .artifact
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::PluginManifest;
    use crate::plugins::PluginId;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const PLUGIN_ID: &str = "plugin-reap-test";

    fn minimal_plugin() -> Plugin {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
id = "plugin-reap-test"
name = "Foo"
description = ""
version = "1.0.0"

[menu]
label = "Foo"
items = []
"#,
        )
        .unwrap();
        Plugin::new(
            PluginId::new(PLUGIN_ID),
            manifest,
            std::path::PathBuf::new(),
        )
    }

    fn spawn_quiet(command: &mut Command) -> Child {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn spawn_short_lived() -> Child {
        let mut command = if cfg!(unix) {
            Command::new("true")
        } else {
            let mut c = Command::new("cmd");
            c.args(["/C", "exit", "0"]);
            c
        };
        spawn_quiet(&mut command)
    }

    fn spawn_long_running() -> Child {
        let mut command = if cfg!(unix) {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        } else {
            let mut c = Command::new("powershell");
            c.args(["-Command", "Start-Sleep", "30"]);
            c
        };
        spawn_quiet(&mut command)
    }

    #[test]
    fn registering_a_daemon_kills_the_one_it_replaces() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let mut plugin = minimal_plugin();
        let first = spawn_long_running();
        let orphan = first.id();
        register_daemon(&mut plugin, first, None);

        register_daemon(&mut plugin, spawn_long_running(), None);

        assert!(
            !crate::process_utils::is_pid_alive(orphan as i32),
            "the replaced daemon must never be left running"
        );
        if let Some(mut child) = plugin.daemon_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    #[test]
    fn daemon_incarnation_advances_when_a_daemon_is_replaced_or_stopped() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let mut plugin = minimal_plugin();
        register_daemon(&mut plugin, spawn_long_running(), None);
        let first_incarnation = current_daemon_incarnation(PLUGIN_ID).unwrap();

        register_daemon(&mut plugin, spawn_long_running(), None);
        let second_incarnation = current_daemon_incarnation(PLUGIN_ID).unwrap();
        assert!(
            second_incarnation > first_incarnation,
            "registering a replacement daemon must advance the tracked incarnation"
        );

        stop_daemon(&mut plugin).unwrap();
        let stopped_incarnation = current_daemon_incarnation(PLUGIN_ID).unwrap();
        assert!(
            stopped_incarnation > second_incarnation,
            "stopping a daemon must invalidate its tracked incarnation"
        );
    }

    #[test]
    fn reap_daemon_if_exited_reaps_dead_child_and_unregisters_pid() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let mut plugin = minimal_plugin();
        let child = spawn_short_lived();
        let pid = child.id();
        super::super::daemon_tracker::registry::register(
            &crate::paths::runtime_pids_dir(),
            plugin.id.as_str(),
            pid,
        );
        plugin.daemon_process = Some(child);

        let deadline = Instant::now() + Duration::from_secs(5);
        while plugin.daemon_pid().is_some() && Instant::now() < deadline {
            reap_daemon_if_exited(&mut plugin);
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            plugin.daemon_pid().is_none(),
            "exited daemon pid {} must be reaped",
            pid
        );
        assert!(
            !crate::paths::runtime_pids_dir()
                .join("plugin-reap-test.pid")
                .exists(),
            "reaping must unregister the tracked pid file"
        );

        reap_daemon_if_exited(&mut plugin);
        assert!(
            plugin.daemon_pid().is_none(),
            "reaping without a daemon process must stay a no-op"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reap_daemon_if_exited_clears_daemon_already_reaped_elsewhere() {
        let mut plugin = minimal_plugin();
        let child = spawn_short_lived();
        let pid = child.id();
        unsafe {
            libc::waitpid(pid as i32, std::ptr::null_mut(), 0);
        }
        plugin.daemon_process = Some(child);

        reap_daemon_if_exited(&mut plugin);

        assert!(
            plugin.daemon_pid().is_none(),
            "daemon pid {} reaped by waitpid(-1) elsewhere must still be cleared",
            pid
        );
    }

    #[test]
    fn waiting_for_stopped_daemons_ends_when_all_exit_or_the_deadline_passes() {
        let polls = std::cell::Cell::new(0);
        let gone = |pid: u32| {
            polls.set(polls.get() + 1);
            pid == 1 || polls.get() > 6
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        assert!(wait_until_gone(vec![1, 2, 3], deadline, gone).is_empty());

        let past = Instant::now();
        assert_eq!(
            wait_until_gone(vec![1, 2, 3], past, |pid| pid == 2),
            vec![1, 3]
        );
        assert!(wait_until_gone(Vec::new(), past, |_| false).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn stopping_several_daemons_shares_one_grace_deadline() {
        let ignore_term = ["-c", "trap '' TERM; exec sleep 30"];
        let daemons: Vec<_> = (0..3)
            .map(|index| StoppingDaemon {
                plugin_id: format!("plugin-stop-{index}"),
                child: spawn_quiet(Command::new("sh").args(ignore_term)),
            })
            .collect();
        let pids: Vec<_> = daemons.iter().map(|daemon| daemon.child.id()).collect();
        std::thread::sleep(Duration::from_millis(100));
        let grace = Duration::from_secs(2);
        let started = Instant::now();

        finish_stops_by(daemons, started + grace);

        let elapsed = started.elapsed();
        assert!(
            elapsed >= grace,
            "a daemon ignoring the stop gets its grace"
        );
        assert!(
            elapsed < grace * 3,
            "graces must overlap, not add up: took {elapsed:?}"
        );
        for pid in pids {
            assert!(!crate::process_utils::is_pid_alive(pid as i32));
        }
    }

    #[test]
    fn reap_daemon_if_exited_keeps_running_child() {
        let mut plugin = minimal_plugin();
        let child = spawn_long_running();
        let pid = child.id();
        plugin.daemon_process = Some(child);

        reap_daemon_if_exited(&mut plugin);

        assert_eq!(
            plugin.daemon_pid(),
            Some(pid),
            "running daemon must not be reaped"
        );

        let child = plugin.daemon_process.as_mut().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[cfg(unix)]
    fn inheriting_plugin() -> Plugin {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
id = "qol-alt-tab"
name = "Foo"
description = ""
version = "1.0.0"

[menu]
label = "Foo"
items = []
"#,
        )
        .unwrap();
        Plugin::new(
            PluginId::new("qol-alt-tab"),
            manifest,
            std::path::PathBuf::new(),
        )
    }

    #[cfg(unix)]
    #[test]
    fn stop_daemon_drops_the_pre_bound_listener() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let mut plugin = inheriting_plugin();
        let socket_path = format!("/tmp/qol-dl-stop-{}.sock", std::process::id());
        let _ = std::fs::remove_file(&socket_path);
        let daemon_config = crate::plugins::manifest::DaemonConfig {
            enabled: true,
            command: "any".to_string(),
            socket: Some(socket_path.clone()),
            port: None,
            extra_ports: Vec::new(),
            inherit_listener: true,
        };
        plugin.daemon_listener = listener::bind_for_plugin(&plugin, &daemon_config);
        assert!(
            plugin.daemon_listener.is_some(),
            "test setup must pre-bind a listener"
        );
        plugin.daemon_process = Some(spawn_long_running());

        stop_daemon(&mut plugin).unwrap();

        assert!(
            plugin.daemon_listener.is_none(),
            "stop_daemon must drop the retained listener so a disabled plugin's socket \
             refuses connections instead of accepting into a backlog nothing will ever \
             read from"
        );
        let _ = std::fs::remove_file(&socket_path);
    }

    #[cfg(unix)]
    #[test]
    fn stop_daemon_drops_the_listener_even_when_the_daemon_already_exited() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let mut plugin = inheriting_plugin();
        let socket_path = format!("/tmp/qol-dl-reaped-{}.sock", std::process::id());
        let _ = std::fs::remove_file(&socket_path);
        let daemon_config = crate::plugins::manifest::DaemonConfig {
            enabled: true,
            command: "any".to_string(),
            socket: Some(socket_path.clone()),
            port: None,
            extra_ports: Vec::new(),
            inherit_listener: true,
        };
        plugin.daemon_listener = listener::bind_for_plugin(&plugin, &daemon_config);
        assert!(
            plugin.daemon_listener.is_some(),
            "test setup must pre-bind a listener"
        );
        plugin.daemon_process = None;

        stop_daemon(&mut plugin).unwrap();

        assert!(
            plugin.daemon_listener.is_none(),
            "a daemon that crashed and was reaped before the stop still leaves the \
             retained listener behind unless stop_daemon drops it unconditionally"
        );
        let _ = std::fs::remove_file(&socket_path);
    }

    #[cfg(unix)]
    fn inheriting_plugin_with_daemon(socket_path: &str) -> Plugin {
        let manifest: PluginManifest = toml::from_str(&format!(
            r#"
[plugin]
id = "qol-alt-tab"
name = "Foo"
description = ""
version = "1.0.0"

[menu]
label = "Foo"
items = []

[daemon]
enabled = true
command = "any"
socket = "{socket_path}"
inherit_listener = true
"#
        ))
        .unwrap();
        Plugin::new(
            PluginId::new("qol-alt-tab"),
            manifest,
            std::path::PathBuf::new(),
        )
    }

    #[cfg(unix)]
    #[test]
    fn existing_daemon_socket_ready_is_not_satisfied_by_the_trays_own_listener() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let socket_path = format!("/tmp/qol-dl-ready-{}.sock", std::process::id());
        let _ = std::fs::remove_file(&socket_path);
        let mut plugin = inheriting_plugin_with_daemon(&socket_path);
        let daemon_config = plugin.manifest.daemon.clone().unwrap();
        plugin.daemon_listener = listener::bind_for_plugin(&plugin, &daemon_config);
        assert!(
            plugin.daemon_listener.is_some(),
            "test setup must pre-bind a listener"
        );

        assert!(
            !existing_daemon_socket_ready(&plugin),
            "the tray's own retained listener accepts connects even with no daemon \
             behind it, so it must not read as a serving daemon - otherwise the \
             supervisor never respawns a crashed one"
        );

        plugin.daemon_process = Some(spawn_long_running());
        assert!(
            existing_daemon_socket_ready(&plugin),
            "a live daemon process behind the retained listener is serving"
        );

        let child = plugin.daemon_process.as_mut().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        let _ = std::fs::remove_file(&socket_path);
    }
}
