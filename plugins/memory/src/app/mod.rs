use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadinessGate, SocketSource};
use qol_runtime::protocol::{DaemonResponse, ReadinessPhase};

use crate::app::warm::WarmState;
use crate::ingest::{self, IngestRoots};
use crate::store::{Store, STALE_TEMP_AGE};

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
    let notes_runs_kept = config.notes_runs_kept as usize;
    let watch_handle =
        match crate::watch::spawn(IngestRoots::resolve(), Arc::clone(&state), notes_runs_kept) {
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
                run_initial_warm(warm_state, progress, notes_runs_kept);
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

fn run_initial_warm(state: Arc<Mutex<WarmState>>, warming: ReadinessGate, notes_runs_kept: usize) {
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
    match crate::distill::run(&store, notes_runs_kept) {
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
    prune_notes_runs_at_warm(&store, notes_runs_kept);
    hygiene_sweep_at_warm(&store);
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

fn prune_notes_runs_at_warm(store: &Store, notes_runs_kept: usize) {
    if notes_runs_kept == 0 {
        return;
    }
    match crate::store::lock::DistillLock::acquire(store, "prune") {
        Ok(None) => {
            qol_runtime::probe!(
                "QOL_MEMORY_DISTILL",
                "event=prune outcome=skip reason=lock_busy"
            );
        }
        Ok(Some(_guard)) => match store.prune_notes_runs(notes_runs_kept) {
            Ok(_) => {}
            Err(error) => {
                eprintln!("qol-memory: notes prune failed: {error:#}");
                qol_runtime::probe!(
                    "QOL_MEMORY_DISTILL",
                    "event=prune outcome=error error={error}"
                );
            }
        },
        Err(error) => {
            eprintln!("qol-memory: notes prune failed: {error:#}");
            qol_runtime::probe!(
                "QOL_MEMORY_DISTILL",
                "event=prune outcome=error error={error}"
            );
        }
    }
}

fn hygiene_sweep_at_warm(store: &Store) {
    let removed_tmp = store.sweep_stale_temp_files(STALE_TEMP_AGE);
    let pruned_markers = match crate::continue_recall::prune_stale_marker_entries(store) {
        Ok(count) => count,
        Err(error) => {
            eprintln!("qol-memory: continue marker prune failed: {error:#}");
            qol_runtime::probe!(
                "QOL_MEMORY_DAEMON",
                "event=marker_prune_failed error={error}"
            );
            0
        }
    };
    qol_runtime::probe!(
        "QOL_MEMORY_DAEMON",
        "event=hygiene_sweep removed_tmp={removed_tmp} pruned_markers={pruned_markers}"
    );
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
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "qol-memory-app-{}-{}-{}",
                tag,
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn seed_run(store: &Store, name: &str) {
        let run = store.notes_root().join(name);
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("notes.jsonl"), "{\"key\":\"n\"}\n").unwrap();
    }

    fn run_count(store: &Store) -> usize {
        std::fs::read_dir(store.notes_root())
            .map(|entries| {
                entries
                    .filter_map(std::result::Result::ok)
                    .filter(|entry| entry.path().is_dir())
                    .count()
            })
            .unwrap_or(0)
    }

    #[test]
    fn prune_notes_runs_at_warm_respects_keep_and_the_lock() {
        let dir = TempDir::new("prune");
        let store = Store::resolve(Some(dir.0.as_path())).unwrap();
        for name in [
            "2026-08-01T09:00:00.000Z",
            "2026-08-02T09:00:00.000Z",
            "2026-08-03T09:00:00.000Z",
            "2026-08-04T09:00:00.000Z",
            "2026-08-05T09:00:00.000Z",
        ] {
            seed_run(&store, name);
        }

        prune_notes_runs_at_warm(&store, 0);
        assert_eq!(run_count(&store), 5);

        prune_notes_runs_at_warm(&store, 2);
        assert_eq!(run_count(&store), 2);

        let held = crate::store::lock::DistillLock::acquire(&store, "test")
            .unwrap()
            .unwrap();
        prune_notes_runs_at_warm(&store, 1);
        assert_eq!(run_count(&store), 2);
        drop(held);
    }

    #[test]
    fn hygiene_sweep_at_warm_removes_stale_temps_and_marker_entries() {
        let dir = TempDir::new("hygiene");
        let store = Store::resolve(Some(dir.0.as_path())).unwrap();
        let stale_temp = dir.0.join(".units.jsonl.abc123.tmp");
        std::fs::write(&stale_temp, b"x").unwrap();
        std::fs::File::open(&stale_temp)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(2 * 60 * 60))
            .unwrap();
        let fresh_temp = dir.0.join(".units.jsonl.def456.tmp");
        std::fs::write(&fresh_temp, b"x").unwrap();
        let marker = serde_json::json!({
            "schema": crate::continue_recall::SCHEMA,
            "cwds": {
                "/old": {
                    "ts": "2000-01-01T00:00:00.000Z",
                    "session": "s",
                    "units_count": 1
                },
                "/recent": {
                    "ts": crate::text::now_iso(),
                    "session": "s",
                    "units_count": 1
                }
            }
        });
        std::fs::write(
            store.continue_marker_path(),
            format!("{}\n", serde_json::to_string_pretty(&marker).unwrap()),
        )
        .unwrap();

        hygiene_sweep_at_warm(&store);

        assert!(!stale_temp.exists());
        assert!(fresh_temp.exists());
        let text = std::fs::read_to_string(store.continue_marker_path()).unwrap();
        let marker: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(marker["cwds"].get("/old").is_none());
        assert!(marker["cwds"]["/recent"].get("last_seen").is_some());
    }

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
