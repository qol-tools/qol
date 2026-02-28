pub mod action_executor;
pub mod action_transport;
pub mod config;
pub mod daemon_tracker;
pub mod loader;
pub mod log_control;
pub mod manager;
pub mod manifest;
pub mod resolver;

pub use config::PluginConfigManager;
pub use loader::PluginLoader;
pub use manager::PluginManager;
pub use manifest::{ActionType, MenuItem, PluginManifest};

use anyhow::Result;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

#[derive(Debug)]
pub struct Plugin {
    pub id: String,
    pub manifest: PluginManifest,
    pub path: PathBuf,
    daemon_process: Option<Child>,
}

impl Plugin {
    pub fn new(id: String, manifest: PluginManifest, path: PathBuf) -> Self {
        Self {
            id,
            manifest,
            path,
            daemon_process: None,
        }
    }

    pub fn start_daemon(&mut self) -> Result<()> {
        let Some(daemon_config) = &self.manifest.daemon else {
            return Ok(());
        };

        if !daemon_config.enabled {
            return Ok(());
        }

        let daemon_path = resolve_plugin_command_path(&self.path, &daemon_config.command)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Daemon executable not found for command {:?} in {:?}",
                    daemon_config.command,
                    self.path
                )
            })?;

        log::info!("Starting daemon for plugin: {}", self.id);
        let mut cmd = Command::new(&daemon_path);
        cmd.current_dir(&self.path).stdin(Stdio::null());

        let log_control = crate::plugins::log_control::load_control_from_shared_config(&self.id);
        let mut relay_patterns = Vec::new();
        if log_control.muted {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        } else if log_control.suppress_patterns.is_empty() {
            cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        } else {
            relay_patterns = log_control.suppress_patterns;
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        }

        if let Some(socket) = daemon_config.socket.as_deref() {
            cmd.env("QOL_TRAY_DAEMON_SOCKET", socket);
        }
        cmd.env("QOL_TRAY_DAEMON_REPLACE_EXISTING", "1");
        cmd.env("QOL_TRAY_STATE_SOCKET", crate::paths::STATE_SOCKET_PATH);

        #[cfg(feature = "dev")]
        cmd.env("RUST_LOG", "debug");

        #[cfg(not(feature = "dev"))]
        cmd.env("RUST_LOG", "warn");

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Run daemon in its own process group so stop_daemon() can kill all children.
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }

        let mut child = cmd.spawn()?;
        if !relay_patterns.is_empty() {
            attach_filtered_log_relay(&self.id, &mut child, relay_patterns);
        }

        if let Some(socket) = daemon_config.socket.as_deref() {
            if !wait_for_socket(socket, &mut child) {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!(
                    "Daemon for {} failed to bind socket within timeout",
                    self.id
                );
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(100));
            match child.try_wait()? {
                Some(status) if !status.success() => {
                    anyhow::bail!("Daemon for {} exited immediately with {}", self.id, status);
                }
                _ => {}
            }
        }

        let pid = child.id();
        self.daemon_process = Some(child);
        crate::os::display::add_ignore_pid(pid);
        crate::signal::register_daemon_pid(pid);
        log::info!("Registered ignore pid {} for plugin {}", pid, self.id);
        Ok(())
    }

    pub fn daemon_pid(&self) -> Option<u32> {
        self.daemon_process.as_ref().map(|c| c.id())
    }

    pub fn stop_daemon(&mut self) -> Result<()> {
        let Some(mut child) = self.daemon_process.take() else {
            return Ok(());
        };

        log::info!("Stopping daemon for plugin: {}", self.id);
        crate::signal::unregister_daemon_pid(child.id());

        #[cfg(unix)]
        unsafe {
            // Kill the entire process group (setsid makes pid == pgid)
            libc::kill(-(child.id() as i32), libc::SIGTERM);
        }
        #[cfg(not(unix))]
        {
            let _ = child.kill();
        }

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(2);

        loop {
            match child.try_wait()? {
                Some(_) => return Ok(()),
                None if start.elapsed() >= timeout => {
                    log::warn!(
                        "Daemon for {} didn't exit gracefully, forcing kill",
                        self.id
                    );
                    child.kill()?;
                    child.wait()?;
                    return Ok(());
                }
                None => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
    }
}

fn attach_filtered_log_relay(plugin_id: &str, child: &mut Child, suppress_patterns: Vec<String>) {
    let active_patterns: Vec<String> = suppress_patterns
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect();

    let patterns = if active_patterns.is_empty() {
        None
    } else {
        Some(Arc::new(active_patterns))
    };

    if let Some(stdout) = child.stdout.take() {
        spawn_log_relay(
            plugin_id.to_string(),
            "stdout",
            stdout,
            patterns.clone(),
            false,
        );
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_log_relay(plugin_id.to_string(), "stderr", stderr, patterns, true);
    }
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

            if let Some(ref patterns) = suppress_patterns {
                let trimmed = line.trim_end();
                if patterns.iter().any(|p| trimmed.contains(p.as_str())) {
                    continue;
                }
            }

            if to_stderr {
                eprint!("{}", line);
            } else {
                print!("{}", line);
            }
        }
    });
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.stop_daemon();
    }
}

fn wait_for_socket(socket_path: &str, child: &mut Child) -> bool {
    let path = Path::new(socket_path);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let poll_interval = std::time::Duration::from_millis(50);

    while std::time::Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            log::error!("Daemon exited early with {}", status);
            return false;
        }
        if path.exists() {
            #[cfg(unix)]
            {
                if std::os::unix::net::UnixStream::connect(path).is_ok() {
                    return true;
                }
            }
            #[cfg(not(unix))]
            {
                return true;
            }
        }
        std::thread::sleep(poll_interval);
    }
    false
}

pub(crate) fn resolve_plugin_command_path(plugin_dir: &Path, command: &str) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.components().count() != 1 {
        return None;
    }

    let canonical_plugin_dir = std::fs::canonicalize(plugin_dir).ok()?;
    let is_allowed_candidate = |path: &Path| -> bool {
        if !path.is_file() {
            return false;
        }
        std::fs::canonicalize(path)
            .ok()
            .is_some_and(|resolved| resolved.starts_with(&canonical_plugin_dir))
    };

    #[cfg(feature = "dev")]
    {
        let debug_target = plugin_dir
            .join("target")
            .join("debug")
            .join(command_path.as_os_str());
        if is_allowed_candidate(&debug_target) {
            return Some(debug_target);
        }
        let release_target = plugin_dir
            .join("target")
            .join("release")
            .join(command_path.as_os_str());
        if is_allowed_candidate(&release_target) {
            return Some(release_target);
        }
    }

    let primary = plugin_dir.join(command_path.as_os_str());
    if is_allowed_candidate(&primary) {
        return Some(primary);
    }

    #[cfg(windows)]
    if primary.extension().is_none() {
        let exe_candidate = primary.with_extension("exe");
        if is_allowed_candidate(&exe_candidate) {
            return Some(exe_candidate);
        }
    }

    None
}

#[derive(Debug)]
pub(crate) struct MissingBinaryContractError {
    plugin_id: String,
    plugin_path: PathBuf,
    command_field: &'static str,
    command: String,
}

impl std::fmt::Display for MissingBinaryContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {:?} binary not found for plugin {} in {:?}",
            self.command_field, self.command, self.plugin_id, self.plugin_path
        )
    }
}

impl std::error::Error for MissingBinaryContractError {}

pub(crate) fn validate_execution_contract(
    plugin_id: &str,
    manifest: &PluginManifest,
    plugin_path: &Path,
) -> Result<()> {
    ensure_command_binary_exists(
        plugin_id,
        plugin_path,
        "runtime.command",
        manifest
            .runtime
            .as_ref()
            .map(|runtime| runtime.command.as_str()),
    )?;
    ensure_command_binary_exists(
        plugin_id,
        plugin_path,
        "daemon.command",
        manifest
            .daemon
            .as_ref()
            .filter(|daemon| daemon.enabled)
            .map(|daemon| daemon.command.as_str()),
    )?;
    Ok(())
}

fn ensure_command_binary_exists(
    plugin_id: &str,
    plugin_path: &Path,
    command_field: &'static str,
    command: Option<&str>,
) -> Result<()> {
    let Some(command) = command else {
        return Ok(());
    };
    if resolve_plugin_command_path(plugin_path, command).is_some() {
        return Ok(());
    }
    Err(MissingBinaryContractError {
        plugin_id: plugin_id.to_string(),
        plugin_path: plugin_path.to_path_buf(),
        command_field,
        command: command.to_string(),
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::resolve_plugin_command_path;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolve_plugin_command_path_rejects_nested_command() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("binary"), "").unwrap();

        let resolved = resolve_plugin_command_path(temp_dir.path(), "nested/binary");
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_plugin_command_path_resolves_regular_file() {
        let temp_dir = TempDir::new().unwrap();
        let binary = temp_dir.path().join("binary");
        fs::write(&binary, "").unwrap();

        let resolved = resolve_plugin_command_path(temp_dir.path(), "binary");
        assert_eq!(resolved, Some(binary));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_plugin_command_path_rejects_symlink_escape() {
        let temp_dir = TempDir::new().unwrap();
        let outside_dir = TempDir::new().unwrap();
        let outside_binary = outside_dir.path().join("outside-binary");
        fs::write(&outside_binary, "").unwrap();

        let escaped = temp_dir.path().join("binary");
        std::os::unix::fs::symlink(&outside_binary, &escaped).unwrap();

        let resolved = resolve_plugin_command_path(temp_dir.path(), "binary");
        assert!(resolved.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_plugin_command_path_allows_internal_symlink() {
        let temp_dir = TempDir::new().unwrap();
        let real_binary = temp_dir.path().join("real-binary");
        fs::write(&real_binary, "").unwrap();

        let linked_binary = temp_dir.path().join("binary");
        std::os::unix::fs::symlink(&real_binary, &linked_binary).unwrap();

        let resolved = resolve_plugin_command_path(temp_dir.path(), "binary");
        assert_eq!(resolved, Some(linked_binary));
    }
}
