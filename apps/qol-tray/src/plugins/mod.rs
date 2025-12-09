pub mod manifest;
pub mod loader;
pub mod manager;
pub mod config;

pub use manifest::{PluginManifest, MenuItem, ActionType};
pub use loader::PluginLoader;
pub use manager::PluginManager;
pub use config::PluginConfigManager;

use anyhow::Result;
use std::path::PathBuf;
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

        let daemon_path = self.path.join(&daemon_config.command);
        if !daemon_path.exists() {
            anyhow::bail!("Daemon executable not found: {:?}", daemon_path);
        }

        log::info!("Starting daemon for plugin: {}", self.id);
        let mut child = Command::new(&daemon_path)
            .current_dir(&self.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;

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
