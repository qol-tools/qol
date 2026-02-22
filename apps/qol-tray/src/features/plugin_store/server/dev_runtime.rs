#![cfg(feature = "dev")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use crate::daemon::{DaemonEvent, EventBus};
use crate::dev;
use crate::dev::state::BuildResultInfo;

use super::types::{
    BuildProgressSnapshot, BuildStateResponse, MockTargetInfo, MOCK_TARGET_PLUGIN_BUILD,
    MOCK_TARGET_SELF_RECOMPILE, MOCK_TARGET_SELF_UPDATE,
};

pub(super) static BUILD_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static MOCK_BUILD_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static MOCK_BUILD_CANCEL: AtomicBool = AtomicBool::new(false);
static MOCK_UPDATE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static MOCK_UPDATE_CANCEL: AtomicBool = AtomicBool::new(false);
static MOCK_RECOMPILE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static MOCK_RECOMPILE_CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Default)]
struct BuildStateStore {
    building: bool,
    progress: HashMap<String, BuildProgressSnapshot>,
}

static BUILD_STATE_STORE: LazyLock<Mutex<BuildStateStore>> =
    LazyLock::new(|| Mutex::new(BuildStateStore::default()));

pub(super) fn mark_build_state_started() {
    let mut store = match BUILD_STATE_STORE.lock() {
        Ok(guard) => guard,
        Err(error) => {
            log::error!("Build state lock poisoned while marking start: {}", error);
            return;
        }
    };
    store.building = true;
    store.progress.clear();
}

pub(super) fn mark_build_state_progress(plugin_id: &str, status: &str, percent: u8, phase: &str) {
    let mut store = match BUILD_STATE_STORE.lock() {
        Ok(guard) => guard,
        Err(error) => {
            log::error!(
                "Build state lock poisoned while updating progress: {}",
                error
            );
            return;
        }
    };

    if !store.building {
        store.building = true;
    }
    store.progress.insert(
        plugin_id.to_string(),
        BuildProgressSnapshot {
            status: status.to_string(),
            percent,
            phase: phase.to_string(),
        },
    );
}

pub(super) fn mark_build_state_finished() {
    let mut store = match BUILD_STATE_STORE.lock() {
        Ok(guard) => guard,
        Err(error) => {
            log::error!("Build state lock poisoned while marking finish: {}", error);
            return;
        }
    };
    store.building = false;
    store.progress.clear();
}

pub(super) fn read_build_state_snapshot() -> BuildStateResponse {
    let atomic_building = BUILD_IN_PROGRESS.load(Ordering::SeqCst);

    let store = match BUILD_STATE_STORE.lock() {
        Ok(guard) => guard,
        Err(error) => {
            log::error!(
                "Build state lock poisoned while reading snapshot: {}",
                error
            );
            return BuildStateResponse {
                building: atomic_building,
                progress: HashMap::new(),
            };
        }
    };

    let building = atomic_building || store.building;
    let progress = if building {
        store.progress.clone()
    } else {
        HashMap::new()
    };
    BuildStateResponse { building, progress }
}

pub(super) fn mock_target_infos() -> Vec<MockTargetInfo> {
    vec![
        MockTargetInfo {
            id: MOCK_TARGET_SELF_UPDATE,
            label: "Self Update",
            running: MOCK_UPDATE_IN_PROGRESS.load(Ordering::SeqCst),
            supports_stop: true,
        },
        MockTargetInfo {
            id: MOCK_TARGET_SELF_RECOMPILE,
            label: "Self Recompile",
            running: MOCK_RECOMPILE_IN_PROGRESS.load(Ordering::SeqCst),
            supports_stop: true,
        },
        MockTargetInfo {
            id: MOCK_TARGET_PLUGIN_BUILD,
            label: "Plugin Build",
            running: MOCK_BUILD_IN_PROGRESS.load(Ordering::SeqCst),
            supports_stop: true,
        },
    ]
}

pub(super) fn any_mock_target_running() -> bool {
    MOCK_UPDATE_IN_PROGRESS.load(Ordering::SeqCst)
        || MOCK_RECOMPILE_IN_PROGRESS.load(Ordering::SeqCst)
        || MOCK_BUILD_IN_PROGRESS.load(Ordering::SeqCst)
}

pub(super) fn start_mock_self_update(events: Arc<EventBus>) -> Result<(), &'static str> {
    if MOCK_UPDATE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return Err("Mock self-update already in progress");
    }
    MOCK_UPDATE_CANCEL.store(false, Ordering::SeqCst);

    tokio::spawn(async move {
        struct MockUpdateGuard;
        impl Drop for MockUpdateGuard {
            fn drop(&mut self) {
                MOCK_UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
                MOCK_UPDATE_CANCEL.store(false, Ordering::SeqCst);
            }
        }
        let _guard = MockUpdateGuard;

        for i in 0..=100u8 {
            if MOCK_UPDATE_CANCEL.load(Ordering::SeqCst) {
                break;
            }
            events.send(DaemonEvent::UpdateProgress { percent: i });
            tokio::time::sleep(tokio::time::Duration::from_millis(45)).await;
        }

        events.send(DaemonEvent::UpdateComplete);
    });

    Ok(())
}

pub(super) fn stop_mock_self_update_internal() -> bool {
    if !MOCK_UPDATE_IN_PROGRESS.load(Ordering::SeqCst) {
        return false;
    }
    MOCK_UPDATE_CANCEL.store(true, Ordering::SeqCst);
    true
}

fn mock_recompile_phase(percent: u8) -> &'static str {
    match percent {
        0..=10 => "Preparing build",
        11..=35 => "Resolving dependencies",
        36..=95 => "Compiling crates",
        _ => "Finalizing build",
    }
}

pub(super) fn start_mock_self_recompile(events: Arc<EventBus>) -> Result<(), &'static str> {
    if MOCK_RECOMPILE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return Err("Mock self-recompile already in progress");
    }
    MOCK_RECOMPILE_CANCEL.store(false, Ordering::SeqCst);

    tokio::spawn(async move {
        struct MockRecompileGuard;
        impl Drop for MockRecompileGuard {
            fn drop(&mut self) {
                MOCK_RECOMPILE_IN_PROGRESS.store(false, Ordering::SeqCst);
                MOCK_RECOMPILE_CANCEL.store(false, Ordering::SeqCst);
            }
        }
        let _guard = MockRecompileGuard;

        for i in 0..=100u8 {
            if MOCK_RECOMPILE_CANCEL.load(Ordering::SeqCst) {
                break;
            }
            events.send(DaemonEvent::SelfRecompileProgress {
                percent: i,
                phase: mock_recompile_phase(i).to_string(),
            });
            tokio::time::sleep(tokio::time::Duration::from_millis(45)).await;
        }

        events.send(DaemonEvent::SelfRecompileComplete);
    });

    Ok(())
}

pub(super) fn stop_mock_self_recompile_internal() -> bool {
    if !MOCK_RECOMPILE_IN_PROGRESS.load(Ordering::SeqCst) {
        return false;
    }
    MOCK_RECOMPILE_CANCEL.store(true, Ordering::SeqCst);
    true
}

pub(super) fn start_mock_plugin_build(
    events: Arc<EventBus>,
    config_dir: Option<std::path::PathBuf>,
) -> Result<(), &'static str> {
    if MOCK_BUILD_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return Err("Mock plugin build already in progress");
    }
    MOCK_BUILD_CANCEL.store(false, Ordering::SeqCst);

    tokio::spawn(async move {
        struct MockBuildGuard;
        impl Drop for MockBuildGuard {
            fn drop(&mut self) {
                MOCK_BUILD_IN_PROGRESS.store(false, Ordering::SeqCst);
                MOCK_BUILD_CANCEL.store(false, Ordering::SeqCst);
                mark_build_state_finished();
            }
        }
        let _guard = MockBuildGuard;

        let mut plugin_ids: Vec<String> = config_dir
            .as_deref()
            .map(dev::load_dev_links)
            .unwrap_or_default()
            .into_keys()
            .collect();
        plugin_ids.sort();

        if MOCK_BUILD_CANCEL.load(Ordering::SeqCst) {
            mark_build_state_finished();
            events.send(DaemonEvent::BuildComplete { results: vec![] });
            return;
        }

        mark_build_state_started();
        events.send(DaemonEvent::BuildStarted);
        if plugin_ids.is_empty() {
            mark_build_state_finished();
            events.send(DaemonEvent::BuildComplete { results: vec![] });
            return;
        }

        for plugin_id in &plugin_ids {
            mark_build_state_progress(plugin_id, "queued", 0, "Queued");
            events.send(DaemonEvent::BuildPluginProgress {
                plugin_id: plugin_id.clone(),
                status: "queued".to_string(),
                percent: 0,
                phase: "Queued".to_string(),
            });
        }

        for plugin_id in &plugin_ids {
            if MOCK_BUILD_CANCEL.load(Ordering::SeqCst) {
                mark_build_state_finished();
                events.send(DaemonEvent::BuildComplete { results: vec![] });
                return;
            }

            mark_build_state_progress(plugin_id, "building", 0, "0/24 preparing");
            events.send(DaemonEvent::BuildPluginProgress {
                plugin_id: plugin_id.clone(),
                status: "building".to_string(),
                percent: 0,
                phase: "0/24 preparing".to_string(),
            });
            tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;

            for done in 1..=24 {
                if MOCK_BUILD_CANCEL.load(Ordering::SeqCst) {
                    mark_build_state_finished();
                    events.send(DaemonEvent::BuildComplete { results: vec![] });
                    return;
                }

                let percent = ((done * 100) / 24) as u8;
                let phase = format!("{}/24 compiling", done);
                mark_build_state_progress(plugin_id, "building", percent, &phase);
                events.send(DaemonEvent::BuildPluginProgress {
                    plugin_id: plugin_id.clone(),
                    status: "building".to_string(),
                    percent,
                    phase,
                });
                tokio::time::sleep(tokio::time::Duration::from_millis(55)).await;
            }
        }

        let results = plugin_ids
            .into_iter()
            .map(|plugin_id| BuildResultInfo {
                plugin_id,
                success: true,
                output: "Mock build completed".to_string(),
                skipped: false,
            })
            .collect();
        mark_build_state_finished();
        events.send(DaemonEvent::BuildComplete { results });
    });

    Ok(())
}

pub(super) fn stop_mock_plugin_build_internal() -> bool {
    if !MOCK_BUILD_IN_PROGRESS.load(Ordering::SeqCst) {
        return false;
    }
    MOCK_BUILD_CANCEL.store(true, Ordering::SeqCst);
    true
}
