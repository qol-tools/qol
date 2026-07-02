mod spawn;

use super::Plugin;
use anyhow::Result;
use std::process::Child;
use std::time::Duration;

const DAEMON_STOP_GRACE: Duration = Duration::from_secs(2);

pub(super) fn start_daemon(plugin: &mut Plugin) -> Result<()> {
    let Some(daemon_config) = spawn::enabled_daemon(plugin) else {
        return Ok(());
    };

    let child = spawn::spawn_daemon(plugin, daemon_config)?;
    register_daemon(plugin, child);
    Ok(())
}

pub(super) fn existing_daemon_socket_ready(plugin: &Plugin) -> bool {
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
}

fn reaped_elsewhere(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::ECHILD)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

pub(super) fn stop_daemon(plugin: &mut Plugin) -> Result<()> {
    let Some(mut child) = plugin.daemon_process.take() else {
        return Ok(());
    };

    log::info!("Stopping daemon for plugin: {}", plugin.id);
    super::daemon_tracker::registry::unregister(
        &crate::paths::runtime_pids_dir(),
        plugin.id.as_str(),
        child.id(),
    );
    if let Err(error) = crate::process_utils::terminate_owned(&mut child, DAEMON_STOP_GRACE) {
        log::warn!("Error reaping daemon for {}: {}", plugin.id, error);
    }
    Ok(())
}

fn register_daemon(plugin: &mut Plugin, child: Child) {
    let pid = child.id();
    plugin.daemon_process = Some(child);
    #[cfg(unix)]
    crate::desktop_state::add_ignore_pid(pid);
    super::daemon_tracker::registry::register(
        &crate::paths::runtime_pids_dir(),
        plugin.id.as_str(),
        pid,
    );
    log::info!("Registered ignore pid {} for plugin {}", pid, plugin.id);
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
    fn reap_daemon_if_exited_reaps_dead_child_and_unregisters_pid() {
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
}
