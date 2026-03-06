#![cfg(feature = "dev")]

mod plugin_build;
mod self_recompile;

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::daemon::{DaemonEvent, EventBus};
use crate::dev::adapters::{CoreEventSink, DevMockTarget, DevRuntimeStateStore};
use crate::dev::core::{CoreBuildResult, CoreEvent};

use super::core_events;

const MOCK_PROGRESS_DELAY: Duration = Duration::from_millis(45);

pub(super) fn start_mock_self_update(
    state: Arc<dyn DevRuntimeStateStore>,
    events: Arc<EventBus>,
) -> Result<(), &'static str> {
    let task_state = Arc::clone(&state);
    start_mock_target(
        state,
        DevMockTarget::SelfUpdate,
        "Mock self-update already in progress",
        run_mock_self_update(task_state, events),
    )
}

pub(super) fn stop_mock_self_update(state: &dyn DevRuntimeStateStore) -> bool {
    stop_mock_target(state, DevMockTarget::SelfUpdate)
}

async fn run_mock_self_update(state: Arc<dyn DevRuntimeStateStore>, events: Arc<EventBus>) {
    let progress_events = Arc::clone(&events);
    run_percent_task(
        state,
        DevMockTarget::SelfUpdate,
        move |percent| progress_events.send(DaemonEvent::UpdateProgress { percent }),
        move || events.send(DaemonEvent::UpdateComplete),
    )
    .await;
}

pub(super) fn start_mock_self_recompile(
    state: Arc<dyn DevRuntimeStateStore>,
    events: Arc<EventBus>,
) -> Result<(), &'static str> {
    let task_state = Arc::clone(&state);
    start_mock_target(
        state,
        DevMockTarget::SelfRecompile,
        "Mock self-recompile already in progress",
        self_recompile::run(task_state, events),
    )
}

pub(super) fn stop_mock_self_recompile(state: &dyn DevRuntimeStateStore) -> bool {
    stop_mock_target(state, DevMockTarget::SelfRecompile)
}

pub(super) fn start_mock_plugin_build(
    state: Arc<dyn DevRuntimeStateStore>,
    events: Arc<EventBus>,
    config_dir: Option<PathBuf>,
    fallback_plugin_ids: Vec<String>,
) -> Result<(), &'static str> {
    let task_state = Arc::clone(&state);
    start_mock_target(
        state,
        DevMockTarget::PluginBuild,
        "Mock plugin build already in progress",
        plugin_build::run(task_state, events, config_dir, fallback_plugin_ids),
    )
}

pub(super) fn stop_mock_plugin_build(state: &dyn DevRuntimeStateStore) -> bool {
    stop_mock_target(state, DevMockTarget::PluginBuild)
}

fn start_mock_target<F>(
    state: Arc<dyn DevRuntimeStateStore>,
    target: DevMockTarget,
    already_running_error: &'static str,
    task: F,
) -> Result<(), &'static str>
where
    F: Future<Output = ()> + Send + 'static,
{
    if !state.try_start_mock_target(target) {
        return Err(already_running_error);
    }

    tokio::spawn(task);
    Ok(())
}

fn stop_mock_target(state: &dyn DevRuntimeStateStore, target: DevMockTarget) -> bool {
    state.request_stop_mock_target(target)
}

struct MockTargetGuard {
    state: Arc<dyn DevRuntimeStateStore>,
    target: DevMockTarget,
}

impl MockTargetGuard {
    fn new(state: Arc<dyn DevRuntimeStateStore>, target: DevMockTarget) -> Self {
        Self { state, target }
    }
}

impl Drop for MockTargetGuard {
    fn drop(&mut self) {
        self.state.clear_mock_target(self.target);
    }
}

fn enter_mock_target(
    state: Arc<dyn DevRuntimeStateStore>,
    target: DevMockTarget,
) -> MockTargetGuard {
    MockTargetGuard::new(state, target)
}

fn mock_target_cancelled(state: &dyn DevRuntimeStateStore, target: DevMockTarget) -> bool {
    state.mock_target_cancelled(target)
}

async fn run_percent_task<ProgressFn, CompleteFn>(
    state: Arc<dyn DevRuntimeStateStore>,
    target: DevMockTarget,
    mut on_progress: ProgressFn,
    on_complete: CompleteFn,
) where
    ProgressFn: FnMut(u8),
    CompleteFn: FnOnce(),
{
    let _guard = enter_mock_target(Arc::clone(&state), target);

    for percent in 0..=100u8 {
        if mock_target_cancelled(state.as_ref(), target) {
            break;
        }

        on_progress(percent);
        tokio::time::sleep(MOCK_PROGRESS_DELAY).await;
    }

    on_complete();
}

fn new_mock_core_event_sink(
    events: Arc<EventBus>,
    state: Arc<dyn DevRuntimeStateStore>,
) -> Arc<dyn CoreEventSink> {
    core_events::new_runtime_core_event_sink(events, state)
}

fn publish_mock_build_complete(event_sink: &dyn CoreEventSink, results: Vec<CoreBuildResult>) {
    event_sink.publish(CoreEvent::BuildComplete { results });
}
