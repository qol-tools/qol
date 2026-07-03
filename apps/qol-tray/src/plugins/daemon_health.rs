use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonExpectation {
    NotExpected,
    AutostartBlocked,
    Supervised,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PluginRuntimeStatus {
    NotExpected,
    AutostartBlocked,
    OnDemand {
        pid: u32,
    },
    Down {
        consecutive_failures: u32,
        suppressed: bool,
    },
    Probation {
        pid: u32,
        consecutive_failures: u32,
    },
    Stable {
        pid: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginHealth {
    pub plugin_id: String,
    pub status: PluginRuntimeStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct HealthSnapshot {
    #[serde(default)]
    pub tick: u64,
    #[serde(default)]
    pub process_pid: u32,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub bind_port: u16,
    #[serde(default)]
    pub daemon_autostart_held: bool,
    #[serde(default)]
    pub generation_id: Option<String>,
    #[serde(default)]
    pub plugins: Vec<PluginHealth>,
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
    fn status_serde_round_trips_every_variant() {
        let cases = [
            (
                PluginRuntimeStatus::NotExpected,
                r#"{"state":"not_expected"}"#,
            ),
            (
                PluginRuntimeStatus::AutostartBlocked,
                r#"{"state":"autostart_blocked"}"#,
            ),
            (
                PluginRuntimeStatus::OnDemand { pid: 12 },
                r#"{"state":"on_demand","pid":12}"#,
            ),
            (
                PluginRuntimeStatus::Down {
                    consecutive_failures: 5,
                    suppressed: true,
                },
                r#"{"state":"down","consecutive_failures":5,"suppressed":true}"#,
            ),
            (
                PluginRuntimeStatus::Probation {
                    pid: 12,
                    consecutive_failures: 1,
                },
                r#"{"state":"probation","pid":12,"consecutive_failures":1}"#,
            ),
            (
                PluginRuntimeStatus::Stable { pid: 12 },
                r#"{"state":"stable","pid":12}"#,
            ),
        ];
        for (status, expected_json) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected_json, "serialize {status:?}");
            let back: PluginRuntimeStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status, "round trip {expected_json}");
        }
    }

    #[test]
    fn publisher_writes_file_and_watch_consistently() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("daemon-health.json");
        let (tx, rx) = channel();
        let publisher = HealthPublisher::new(tx, 42700, path.clone());

        publisher.publish(
            3,
            vec![PluginHealth {
                plugin_id: "plugin-foo".to_string(),
                status: PluginRuntimeStatus::Stable { pid: 12 },
            }],
        );

        let from_watch = rx.borrow().clone();
        assert_eq!(from_watch.tick, 3, "watch carries the published tick");
        assert_eq!(from_watch.bind_port, 42700);
        assert_eq!(from_watch.process_pid, std::process::id());
        let from_file: HealthSnapshot =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(from_file, from_watch, "both transports carry one snapshot");
    }
}
