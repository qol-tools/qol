use super::Plugin;
use anyhow::Result;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

pub(super) fn start_daemon(plugin: &mut Plugin) -> Result<()> {
    let Some(daemon_config) = enabled_daemon(plugin) else {
        return Ok(());
    };

    let daemon_path = daemon_path(plugin, daemon_config)?;
    let mut child = spawn_daemon(plugin, daemon_config, &daemon_path)?;
    wait_for_daemon_ready(plugin, daemon_config, &mut child)?;
    register_daemon(plugin, child);
    Ok(())
}

pub(super) fn stop_daemon(plugin: &mut Plugin) -> Result<()> {
    let Some(mut child) = plugin.daemon_process.take() else {
        return Ok(());
    };

    log::info!("Stopping daemon for plugin: {}", plugin.id);
    crate::signal::unregister_daemon_pid(child.id());
    terminate_daemon(&mut child);
    wait_for_exit(plugin, &mut child)
}

fn enabled_daemon(plugin: &Plugin) -> Option<&crate::plugins::manifest::DaemonConfig> {
    plugin
        .manifest
        .daemon
        .as_ref()
        .filter(|daemon| daemon.enabled)
}

fn daemon_path(
    plugin: &Plugin,
    daemon_config: &crate::plugins::manifest::DaemonConfig,
) -> Result<PathBuf> {
    super::resolve_plugin_command_path(&plugin.path, &daemon_config.command).ok_or_else(|| {
        anyhow::anyhow!(
            "Daemon executable not found for command {:?} in {:?}",
            daemon_config.command,
            plugin.path
        )
    })
}

fn spawn_daemon(
    plugin: &Plugin,
    daemon_config: &crate::plugins::manifest::DaemonConfig,
    daemon_path: &Path,
) -> Result<Child> {
    log::info!("Starting daemon for plugin: {}", plugin.id);
    let mut command = daemon_command(plugin, daemon_config, daemon_path);
    let relay_patterns = configure_log_relay(plugin, &mut command);
    let mut child = command.spawn()?;
    attach_filtered_log_relay(plugin, &mut child, relay_patterns);
    Ok(child)
}

fn daemon_command(
    plugin: &Plugin,
    daemon_config: &crate::plugins::manifest::DaemonConfig,
    daemon_path: &Path,
) -> Command {
    let mut command = Command::new(daemon_path);
    command.current_dir(&plugin.path).stdin(Stdio::null());
    apply_log_env(&mut command);
    apply_daemon_env(&mut command, daemon_config);
    apply_process_group(&mut command);
    command
}

fn apply_daemon_env(command: &mut Command, daemon_config: &crate::plugins::manifest::DaemonConfig) {
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

fn apply_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
}

fn configure_log_relay(plugin: &Plugin, command: &mut Command) -> Vec<String> {
    let log_control = crate::plugins::log_control::load_control_from_shared_config(&plugin.id);
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

fn attach_filtered_log_relay(plugin: &Plugin, child: &mut Child, suppress_patterns: Vec<String>) {
    let patterns = active_patterns(suppress_patterns);
    if let Some(stdout) = child.stdout.take() {
        spawn_log_relay(plugin.id.clone(), "stdout", stdout, patterns.clone(), false);
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_log_relay(plugin.id.clone(), "stderr", stderr, patterns, true);
    }
}

fn active_patterns(patterns: Vec<String>) -> Option<Arc<Vec<String>>> {
    let active_patterns: Vec<String> = patterns
        .into_iter()
        .filter(|pattern| !pattern.is_empty())
        .collect();
    if active_patterns.is_empty() {
        return None;
    }
    Some(Arc::new(active_patterns))
}

fn spawn_log_relay<R>(
    plugin_id: String,
    stream_name: &'static str,
    reader: R,
    suppress_patterns: Option<Arc<Vec<String>>>,
    to_stderr: bool,
) where
    R: std::io::Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            let read = match reader.read_line(&mut line) {
                Ok(read) => read,
                Err(error) => {
                    log::debug!(
                        "Plugin daemon log relay failed for {} ({}): {}",
                        plugin_id,
                        stream_name,
                        error
                    );
                    break;
                }
            };
            if read == 0 {
                break;
            }
            if should_suppress_line(&line, suppress_patterns.as_ref()) {
                continue;
            }
            print_relay_line(&line, to_stderr);
        }
    });
}

fn should_suppress_line(line: &str, patterns: Option<&Arc<Vec<String>>>) -> bool {
    let Some(patterns) = patterns else {
        return false;
    };
    let trimmed = line.trim_end();
    patterns
        .iter()
        .any(|pattern| trimmed.contains(pattern.as_str()))
}

fn print_relay_line(line: &str, to_stderr: bool) {
    if to_stderr {
        eprint!("{}", line);
        return;
    }
    print!("{}", line);
}

fn wait_for_daemon_ready(
    plugin: &Plugin,
    daemon_config: &crate::plugins::manifest::DaemonConfig,
    child: &mut Child,
) -> Result<()> {
    let Some(socket) = daemon_config.socket.as_deref() else {
        return wait_for_non_socket_start(plugin, child);
    };

    wait_for_socket_start(plugin, child, socket)
}

fn wait_for_socket_start(plugin: &Plugin, child: &mut Child, socket: &str) -> Result<()> {
    if wait_for_socket(socket, child) {
        return Ok(());
    }

    let _ = child.kill();
    let _ = child.wait();
    anyhow::bail!(
        "Daemon for {} failed to bind socket within timeout",
        plugin.id
    )
}

fn wait_for_non_socket_start(plugin: &Plugin, child: &mut Child) -> Result<()> {
    std::thread::sleep(std::time::Duration::from_millis(100));
    let exited = child.try_wait()?;
    let Some(status) = exited else {
        return Ok(());
    };
    if status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "Daemon for {} exited immediately with {}",
        plugin.id,
        status
    )
}

fn register_daemon(plugin: &mut Plugin, child: Child) {
    let pid = child.id();
    plugin.daemon_process = Some(child);
    #[cfg(unix)]
    crate::os::display::add_ignore_pid(pid);
    crate::signal::register_daemon_pid(pid);
    log::info!("Registered ignore pid {} for plugin {}", pid, plugin.id);
}

fn terminate_daemon(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }

    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

fn wait_for_exit(plugin: &Plugin, child: &mut Child) -> Result<()> {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(2);

    loop {
        match child.try_wait()? {
            Some(_) => return Ok(()),
            None if start.elapsed() >= timeout => return force_kill_daemon(plugin, child),
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
}

fn force_kill_daemon(plugin: &Plugin, child: &mut Child) -> Result<()> {
    log::warn!(
        "Daemon for {} didn't exit gracefully, forcing kill",
        plugin.id
    );
    child.kill()?;
    child.wait()?;
    Ok(())
}

fn wait_for_socket(socket_path: &str, child: &mut Child) -> bool {
    let path = Path::new(socket_path);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let poll_interval = std::time::Duration::from_millis(50);

    while std::time::Instant::now() < deadline {
        if daemon_exited(child) {
            return false;
        }
        if socket_reachable(path) {
            return true;
        }
        std::thread::sleep(poll_interval);
    }
    false
}

fn daemon_exited(child: &mut Child) -> bool {
    match child.try_wait() {
        Ok(Some(status)) => {
            log::error!("Daemon exited early with {}", status);
            true
        }
        _ => false,
    }
}

fn socket_reachable(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    #[cfg(unix)]
    {
        return std::os::unix::net::UnixStream::connect(path).is_ok();
    }

    #[cfg(not(unix))]
    {
        true
    }
}
