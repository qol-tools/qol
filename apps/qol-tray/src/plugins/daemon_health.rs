use std::path::{Path, PathBuf};
use tokio::sync::watch;

pub use qol_conventions::dev_health::{HealthSnapshot, PluginHealth, PluginRuntimeStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonExpectation {
    NotExpected,
    AutostartBlocked,
    Supervised,
}

pub fn channel() -> (
    watch::Sender<HealthSnapshot>,
    watch::Receiver<HealthSnapshot>,
) {
    watch::channel(HealthSnapshot::default())
}

pub fn default_file_path() -> PathBuf {
    crate::paths::runtime_dir().join("daemon-health.json")
}

pub struct HealthPublisher {
    tx: watch::Sender<HealthSnapshot>,
    bind_port: u16,
    file_path: PathBuf,
}

impl HealthPublisher {
    pub fn new(tx: watch::Sender<HealthSnapshot>, bind_port: u16, file_path: PathBuf) -> Self {
        Self {
            tx,
            bind_port,
            file_path,
        }
    }

    pub fn publish(&self, tick: u64, plugins: Vec<PluginHealth>) {
        let snapshot = HealthSnapshot {
            tick,
            process_pid: std::process::id(),
            role: if crate::dev_generation::is_shadow() {
                "shadow"
            } else {
                "stable"
            }
            .to_string(),
            bind_port: self.bind_port,
            daemon_autostart_held: crate::dev_generation::daemon_autostart_held(),
            generation_id: crate::dev_generation::current().generation_id(),
            plugins,
        };
        if let Err(error) = write_snapshot_file(&self.file_path, &snapshot) {
            log::warn!("Failed to write daemon health snapshot: {error:#}");
        }
        self.tx.send_replace(snapshot);
    }
}

fn write_snapshot_file(path: &Path, snapshot: &HealthSnapshot) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("health file path has no parent"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(snapshot)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publisher_writes_file_and_watch_consistently() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("daemon-health.json");
        let (tx, rx) = channel();
        let publisher = HealthPublisher::new(tx, qol_conventions::DEFAULT_PORT, path.clone());

        publisher.publish(
            3,
            vec![PluginHealth {
                plugin_id: "plugin-foo".to_string(),
                status: PluginRuntimeStatus::Stable { pid: 12 },
            }],
        );

        let from_watch = rx.borrow().clone();
        assert_eq!(from_watch.tick, 3, "watch carries the published tick");
        assert_eq!(from_watch.bind_port, qol_conventions::DEFAULT_PORT);
        assert_eq!(from_watch.process_pid, std::process::id());
        let from_file: HealthSnapshot =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(from_file, from_watch, "both transports carry one snapshot");
    }
}
