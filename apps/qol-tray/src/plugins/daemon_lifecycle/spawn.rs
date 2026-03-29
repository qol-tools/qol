use crate::plugins::manifest::DaemonConfig;
use crate::plugins::Plugin;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub(super) fn enabled_daemon(plugin: &Plugin) -> Option<&DaemonConfig> {
    plugin
        .manifest
        .daemon
        .as_ref()
        .filter(|daemon| daemon.enabled)
}

pub(super) fn spawn_daemon(plugin: &Plugin, daemon_config: &DaemonConfig) -> Result<Child> {
    let daemon_path = daemon_path(plugin, daemon_config)?;
    let mut command = daemon_command(plugin, daemon_config, &daemon_path);
    #[cfg(feature = "dev")]
    let relay_patterns = configure_log_relay(plugin, &mut command);
    #[cfg(not(feature = "dev"))]
    {
        command.stdout(Stdio::inherit());
        command.stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let version = plugin.manifest.plugin.version.clone();
        let commit = plugin.manifest.build.commit.clone();
        crate::logging::relay::attach_with_prod_log(
            plugin.id.as_str(),
            &version,
            commit.as_deref(),
            child.stderr.take(),
        );
        Ok(child)
    }
    #[cfg(feature = "dev")]
    {
        let mut child = command.spawn()?;
        crate::logging::relay::attach(
            plugin.id.as_str(),
            child.stdout.take(),
            child.stderr.take(),
            relay_patterns,
        );
        Ok(child)
    }
}

fn daemon_path(plugin: &Plugin, daemon_config: &DaemonConfig) -> Result<PathBuf> {
    super::super::resolve_plugin_command_path_for_source(
        &plugin.path,
        &daemon_config.command,
        Some(&plugin.source),
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "Daemon executable not found for command {:?} in {:?}",
            daemon_config.command,
            plugin.path
        )
    })
}

fn daemon_command(plugin: &Plugin, daemon_config: &DaemonConfig, daemon_path: &Path) -> Command {
    let mut command = Command::new(daemon_path);
    command.current_dir(&plugin.path).stdin(Stdio::null());
    apply_log_env(&mut command);
    apply_daemon_env(&mut command, daemon_config);
    apply_process_group(&mut command);
    command
}

fn apply_daemon_env(command: &mut Command, daemon_config: &DaemonConfig) {
    if let Some(socket) = daemon_config.socket.as_deref() {
        command.env("QOL_TRAY_DAEMON_SOCKET", socket);
    }
    command.env("QOL_TRAY_DAEMON_REPLACE_EXISTING", "1");
    command.env("QOL_TRAY_STATE_SOCKET", crate::paths::STATE_SOCKET_PATH);
}

fn apply_log_env(command: &mut Command) {
    #[cfg(feature = "dev")]
    command.env("RUST_LOG", "debug");

    #[cfg(not(feature = "dev"))]
    command.env("RUST_LOG", "warn");
}

fn apply_process_group(_command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        unsafe {
            _command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
}

#[cfg(feature = "dev")]
fn configure_log_relay(plugin: &Plugin, command: &mut Command) -> Vec<String> {
    let log_control = crate::logging::load_plugin_control_from_shared_config(plugin.id.as_str());
    if log_control.muted {
        command.stdout(Stdio::null()).stderr(Stdio::null());
        return Vec::new();
    }
    if log_control.suppress_patterns.is_empty() {
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        return Vec::new();
    }

    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    log_control.suppress_patterns
}
