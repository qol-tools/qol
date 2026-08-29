use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, SocketSource};
use qol_runtime::protocol::DaemonResponse;

use crate::app::warm::WarmState;
use crate::ingest::{self, IngestRoots};
use crate::store::Store;

pub mod request;
pub mod warm;

pub const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

pub fn run_daemon() -> Result<()> {
    let store = Store::resolve(None)?;
    let aliases = crate::aliases::embedded();
    let state = Arc::new(Mutex::new(WarmState::open(store, aliases)?));
    let watch_handle = match crate::watch::spawn(IngestRoots::resolve(), Arc::clone(&state)) {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("qol-memory: transcript watch unavailable: {error}");
            qol_runtime::probe!("QOL_MEMORY_DAEMON", "event=watch_unavailable error={error}");
            None
        }
    };
    let ingest_state = Arc::clone(&state);
    let ingest_roots = IngestRoots::resolve();
    std::thread::Builder::new()
        .name("qol-memory-initial-ingest".to_owned())
        .spawn(move || {
            let paths = ingest::walk_roots(&ingest_roots);
            for chunk in paths.chunks(16) {
                let mut warm = match ingest_state.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let store = warm.store().clone();
                match ingest::ingest_paths(&store, &ingest_roots, chunk, warm.keys()) {
                    Ok(report) if report.appended > 0 => warm.invalidate_layers(),
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("qol-memory: initial ingest failed: {error:#}");
                        qol_runtime::probe!(
                            "QOL_MEMORY_DAEMON",
                            "event=initial_ingest_failed error={error}"
                        );
                    }
                }
            }
            let store = {
                let warm = match ingest_state.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                warm.store().clone()
            };
            match crate::distill::run(&store) {
                Ok(report) if !report.unchanged => {
                    let mut warm = match ingest_state.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    warm.invalidate_layers();
                }
                Ok(_) => {}
                Err(error) if crate::distill::is_busy(&error) => {}
                Err(error) => {
                    eprintln!("qol-memory: initial distill failed: {error:#}");
                    qol_runtime::probe!(
                        "QOL_MEMORY_DAEMON",
                        "event=initial_distill_failed error={error}"
                    );
                }
            }
        })
        .context("failed to start the qol-memory initial ingest thread")?;
    let listen_result =
        core_daemon::run_stateful_request_listener(&DAEMON_CONFIG, state, request::handle)
            .context("qol-memory daemon listener failed");
    drop(watch_handle);
    listen_result
}

pub fn send_request(action: &str, input: serde_json::Value) -> Result<Option<serde_json::Value>> {
    let response =
        core_daemon::send_request(&DAEMON_CONFIG, action, input, Duration::from_secs(10))?;
    match response {
        DaemonResponse::Handled { data } => Ok(data),
        DaemonResponse::Fallback => bail!("qol-memory daemon declined action `{action}`"),
        DaemonResponse::Error { message } => bail!(message),
    }
}

pub fn daemon_unreachable(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(|io_error| {
            matches!(
                io_error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_unreachable_matches_missing_and_refused_sockets() {
        assert!(daemon_unreachable(&anyhow::Error::from(
            std::io::Error::new(std::io::ErrorKind::NotFound, "no socket",)
        )));
        assert!(daemon_unreachable(&anyhow::Error::from(
            std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused",)
        )));
        assert!(!daemon_unreachable(&anyhow::Error::from(
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied",)
        )));
        let wrapped = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no socket",
        ))
        .context("outer");
        assert!(daemon_unreachable(&wrapped));
    }
}
