pub mod manifest;
pub mod loader;
pub mod manager;
pub mod config;
pub mod action_executor;
pub mod action_transport;

pub use manifest::{PluginManifest, MenuItem, ActionType, RuntimeConfig};
pub use loader::PluginLoader;
pub use manager::PluginManager;
pub use config::PluginConfigManager;

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

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
        cmd.current_dir(&self.path)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        if let Some(socket) = daemon_config.socket.as_deref() {
            cmd.env("QOL_TRAY_DAEMON_SOCKET", socket);
        }

        #[cfg(feature = "dev")]
        cmd.env("RUST_LOG", "debug");

        #[cfg(not(feature = "dev"))]
        cmd.env("RUST_LOG", "warn");

        let mut child = cmd.spawn()?;

        std::thread::sleep(std::time::Duration::from_millis(100));

        match child.try_wait()? {
            Some(status) if !status.success() => {
                let stderr = child.stderr.take()
                    .map(|mut s| {
                        let mut buf = String::new();
                        std::io::Read::read_to_string(&mut s, &mut buf).ok();
                        buf
                    })
                    .unwrap_or_default();
                anyhow::bail!("Daemon exited immediately with {}: {}", status, stderr.trim());
            }
            _ => {}
        }

        self.daemon_process = Some(child);
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

        #[cfg(unix)]
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM);
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
                    log::warn!("Daemon for {} didn't exit gracefully, forcing kill", self.id);
                    child.kill()?;
                    child.wait()?;
                    return Ok(());
                }
                None => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.stop_daemon();
    }
}

pub(crate) fn resolve_plugin_command_path(plugin_dir: &Path, command: &str) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.components().count() != 1 {
        return None;
    }

    let mut candidates = vec![plugin_dir.join(command_path.as_os_str())];

    #[cfg(windows)]
    {
        let with_exe: Vec<PathBuf> = candidates
            .iter()
            .filter(|path| path.extension().is_none())
            .map(|path| path.with_extension("exe"))
            .collect();
        candidates.extend(with_exe);
    }

    candidates.into_iter().find(|path| path.is_file())
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
        manifest.runtime.as_ref().map(|runtime| runtime.command.as_str()),
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
