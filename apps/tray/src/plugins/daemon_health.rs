use qol_runtime::protocol::ReadinessPhase;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::watch;

pub use qol_conventions::dev_health::{HealthSnapshot, PluginHealth, PluginRuntimeStatus};

const READINESS_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DaemonReadiness {
    Unknown,
    Serving,
    NotReady {
        phase: ReadinessPhase,
        detail: Option<String>,
    },
}

pub(crate) fn probe_daemon_readiness(socket: &Path) -> DaemonReadiness {
    probe_daemon_readiness_with_timeout(socket, READINESS_PROBE_TIMEOUT)
}

pub(crate) fn probe_daemon_readiness_with_timeout(
    socket: &Path,
    timeout: Duration,
) -> DaemonReadiness {
    platform_readiness(socket, timeout)
}

#[cfg(unix)]
fn platform_readiness(socket: &Path, timeout: Duration) -> DaemonReadiness {
    unix_readiness(socket, timeout)
}

#[cfg(not(unix))]
fn platform_readiness(_socket: &Path, _timeout: Duration) -> DaemonReadiness {
    DaemonReadiness::Unknown
}

#[cfg(unix)]
fn unix_readiness(socket: &Path, timeout: Duration) -> DaemonReadiness {
    use qol_runtime::protocol::{DaemonRequest, DaemonResponse};
    use std::io::{BufRead, BufReader, Write};
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    let Ok(mut payload) = serde_json::to_string(&DaemonRequest {
        action: "ping".to_string(),
        input: serde_json::Value::Null,
    }) else {
        return DaemonReadiness::Unknown;
    };
    payload.push('\n');
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return DaemonReadiness::Unknown;
    };
    if qol_runtime::local_ipc::authorize_peer(&stream).is_err() {
        return DaemonReadiness::Unknown;
    }
    let _ = stream.set_write_timeout(Some(timeout));
    let _ = stream.set_read_timeout(Some(timeout));
    if stream.write_all(payload.as_bytes()).is_err() {
        return DaemonReadiness::Unknown;
    }
    let _ = stream.shutdown(Shutdown::Write);
    let mut line = String::new();
    let answered = BufReader::new(stream)
        .read_line(&mut line)
        .map(|read| read > 0)
        .unwrap_or(false);
    if !answered {
        return DaemonReadiness::Unknown;
    }
    match serde_json::from_str::<DaemonResponse>(line.trim()) {
        Ok(DaemonResponse::Handled { .. }) => DaemonReadiness::Serving,
        Ok(DaemonResponse::NotReady { phase, detail }) => {
            DaemonReadiness::NotReady { phase, detail }
        }
        _ => DaemonReadiness::Unknown,
    }
}

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
    crate::file_io::atomic_write(path, &serde_json::to_vec(snapshot)?)
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

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    fn serve_one_response(socket: std::path::PathBuf, response: &'static str) {
        let listener = UnixListener::bind(&socket).unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            let _ = BufReader::new(&stream).read_line(&mut line);
            let _ = stream.write_all(format!("{response}\n").as_bytes());
        });
    }

    #[test]
    fn readiness_probe_reports_not_ready_with_its_phase_and_detail() {
        let dir = tempfile::TempDir::new().unwrap();
        let socket = dir.path().join("not-ready.sock");
        serve_one_response(
            socket.clone(),
            r#"{"status":"not_ready","phase":"warming","detail":"ingesting transcripts 320/900"}"#,
        );

        assert_eq!(
            probe_daemon_readiness(&socket),
            DaemonReadiness::NotReady {
                phase: ReadinessPhase::Warming,
                detail: Some("ingesting transcripts 320/900".to_string()),
            }
        );
    }

    #[test]
    fn readiness_probe_reports_serving_for_a_handled_ping() {
        let dir = tempfile::TempDir::new().unwrap();
        let socket = dir.path().join("serving.sock");
        serve_one_response(socket.clone(), r#"{"status":"handled"}"#);

        assert_eq!(probe_daemon_readiness(&socket), DaemonReadiness::Serving);
    }

    #[test]
    fn readiness_probe_reports_unknown_when_no_daemon_listens() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(
            probe_daemon_readiness(&dir.path().join("absent.sock")),
            DaemonReadiness::Unknown
        );
    }

    #[test]
    fn readiness_probe_times_out_against_a_silent_listener() {
        let dir = tempfile::TempDir::new().unwrap();
        let socket = dir.path().join("silent.sock");
        let _listener = UnixListener::bind(&socket).unwrap();

        assert_eq!(
            probe_daemon_readiness_with_timeout(&socket, Duration::from_millis(50)),
            DaemonReadiness::Unknown
        );
    }
}

/// Process-wide map of plugins that are running but not yet serving.
///
/// The supervisor writes it on every readiness pass and the dashboard reads it
/// over `/api/readiness`, so a page loaded mid-warmup sees the same deferred
/// state that later arrives live as a `readiness_changed` event.
#[derive(Default)]
pub struct ReadinessRegistry {
    warming: std::sync::Mutex<std::collections::HashMap<String, PluginRuntimeStatus>>,
}

static READINESS: std::sync::OnceLock<ReadinessRegistry> = std::sync::OnceLock::new();

impl ReadinessRegistry {
    pub fn shared() -> &'static Self {
        READINESS.get_or_init(ReadinessRegistry::default)
    }

    pub fn snapshot(&self) -> std::collections::HashMap<String, PluginRuntimeStatus> {
        match self.warming.lock() {
            Ok(warming) => warming.clone(),
            Err(_) => std::collections::HashMap::new(),
        }
    }

    /// Replaces the warming set with the `Starting` plugins of `projected` and
    /// broadcasts one event per plugin whose readiness actually changed.
    pub(crate) fn reconcile(&self, projected: &[PluginHealth]) {
        let next: std::collections::HashMap<String, PluginRuntimeStatus> = projected
            .iter()
            .filter(|health| matches!(health.status, PluginRuntimeStatus::Starting { .. }))
            .map(|health| (health.plugin_id.clone(), health.status.clone()))
            .collect();
        let Ok(mut warming) = self.warming.lock() else {
            return;
        };
        if *warming == next {
            return;
        }
        let mut changed: Vec<(String, Option<PluginRuntimeStatus>)> = next
            .iter()
            .filter(|(id, status)| warming.get(*id) != Some(status))
            .map(|(id, status)| (id.clone(), Some(status.clone())))
            .collect();
        changed.extend(
            warming
                .keys()
                .filter(|id| !next.contains_key(*id))
                .map(|id| (id.clone(), None)),
        );
        *warming = next;
        drop(warming);
        let Some(events) = crate::runtime::events() else {
            return;
        };
        for (plugin_id, runtime_status) in changed {
            events.send(crate::daemon::DaemonEvent::ReadinessChanged {
                plugin_id,
                runtime_status,
            });
        }
    }
}
