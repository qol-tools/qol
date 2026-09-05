use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadinessGate, SocketSource};
use qol_runtime::protocol::{DaemonResponse, ReadinessPhase};

use crate::app::warm::WarmState;
use crate::ingest::{self, IngestRoots};
use crate::store::Store;

pub mod request;
pub mod warm;

#[cfg(test)]
mod verification_tests;

pub const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

pub fn run_daemon() -> Result<()> {
    let store = Store::resolve(None)?;
    let aliases = crate::aliases::embedded();
    let config = crate::config::load();
    let mut warm = WarmState::open(store, aliases)?;
    if config.verify_answers {
        let enabled = crate::verification::ollama::Ollama::new(
            warm.store().root().join("verification"),
            &config.verifier_endpoint,
        )
        .and_then(|provider| warm.enable_verification(provider));
        if let Err(error) = enabled {
            eprintln!("qol-memory: answer verification unavailable: {error:#}");
        }
    }
    let state = Arc::new(Mutex::new(warm));
    let watch_handle = match crate::watch::spawn(IngestRoots::resolve(), Arc::clone(&state)) {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("qol-memory: transcript watch unavailable: {error}");
            qol_runtime::probe!("QOL_MEMORY_DAEMON", "event=watch_unavailable error={error}");
            None
        }
    };
    let readiness = ReadinessGate::starting();
    let warm_state = Arc::clone(&state);
    let warm_readiness = readiness.clone();
    std::thread::Builder::new()
        .name("qol-memory-initial-warm".to_owned())
        .spawn(move || {
            let progress = warm_readiness.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                run_initial_warm(warm_state, progress);
            }));
            if result.is_err() {
                eprintln!("qol-memory: initial warm thread panicked");
            }
            warm_readiness.mark_ready();
        })
        .context("failed to start the qol-memory initial warm thread")?;
    let listener_state = Arc::clone(&state);
    let listen_result = core_daemon::run_stateful_request_listener_with_readiness(
        &DAEMON_CONFIG,
        &readiness,
        listener_state,
        request::handle,
    )
    .context("qol-memory daemon listener failed");
    drop(watch_handle);
    listen_result
}

fn run_initial_warm(state: Arc<Mutex<WarmState>>, warming: ReadinessGate) {
    let roots = IngestRoots::resolve();
    let paths = ingest::walk_roots(&roots);
    let total = paths.len();
    let mut ingested = 0usize;
    warming.set_phase(
        ReadinessPhase::Warming,
        Some(format!("ingesting transcripts 0/{total}")),
    );
    for chunk in paths.chunks(16) {
        let mut warm = match state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let store = warm.store().clone();
        match ingest::ingest_paths(&store, &roots, chunk, warm.keys()) {
            Ok(_) => {}
            Err(error) => {
                eprintln!("qol-memory: initial ingest failed: {error:#}");
                qol_runtime::probe!(
                    "QOL_MEMORY_DAEMON",
                    "event=initial_ingest_failed error={error}"
                );
            }
        }
        ingested += chunk.len();
        warming.set_phase(
            ReadinessPhase::Warming,
            Some(format!("ingesting transcripts {ingested}/{total}")),
        );
    }
    warming.set_phase(ReadinessPhase::Warming, Some("distilling notes".to_owned()));
    let store = {
        let warm = match state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        warm.store().clone()
    };
    match crate::distill::run(&store) {
        Ok(report) if !report.unchanged => {
            let mut warm = match state.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            warm.invalidate_notes_index();
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
    warming.set_phase(
        ReadinessPhase::Warming,
        Some("building warm index".to_owned()),
    );
    {
        let mut warm = match state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Err(error) = warm.layers() {
            eprintln!("qol-memory: initial warm build failed: {error:#}");
            qol_runtime::probe!(
                "QOL_MEMORY_DAEMON",
                "event=initial_warm_failed error={error}"
            );
        }
    }
}

pub fn send_request(action: &str, input: serde_json::Value) -> Result<Option<serde_json::Value>> {
    let response =
        core_daemon::send_request(&DAEMON_CONFIG, action, input, Duration::from_secs(10))?;
    match response {
        DaemonResponse::Handled { data } => Ok(data),
        DaemonResponse::Fallback => bail!("qol-memory daemon declined action `{action}`"),
        DaemonResponse::Error { message } => bail!(message),
        DaemonResponse::NotReady { phase, detail } => {
            let detail = detail.map(|text| format!(": {text}")).unwrap_or_default();
            bail!("qol-memory daemon is not ready ({phase:?}{detail})")
        }
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
