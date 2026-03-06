use crate::dev::core::{self, BuildStatus, CoreBuildResult, CoreInput, CoreState};

use super::super::types::{BuildResult, PluginBuildProgress};

pub(super) struct CoreEventEmitter<F>
where
    F: FnMut(core::CoreEvent),
{
    core_state: CoreState,
    on_event: F,
}

impl<F> CoreEventEmitter<F>
where
    F: FnMut(core::CoreEvent),
{
    pub(super) fn new(on_event: F) -> Self {
        Self {
            core_state: CoreState::default(),
            on_event,
        }
    }

    pub(super) fn run_started(&mut self) {
        self.emit(CoreInput::RunStarted);
    }

    pub(super) fn plugin_progress(
        &mut self,
        plugin_id: &str,
        status: BuildStatus,
        percent: u8,
        phase: &str,
    ) {
        self.emit(CoreInput::PluginProgress {
            plugin_id: plugin_id.to_string(),
            status,
            percent,
            phase: phase.to_string(),
        });
    }

    pub(super) fn run_finished(&mut self, results: &[BuildResult]) {
        let results = results.iter().map(to_core_result).collect();
        self.emit(CoreInput::RunFinished { results });
    }

    fn emit(&mut self, input: CoreInput) {
        let (next_state, events) = core::reduce(std::mem::take(&mut self.core_state), input);
        self.core_state = next_state;
        for event in events {
            (self.on_event)(event);
        }
    }
}

pub(super) fn emit_plugin_progress<F>(event: core::CoreEvent, on_progress: &mut F)
where
    F: FnMut(PluginBuildProgress),
{
    let core::CoreEvent::BuildPluginProgress {
        plugin_id,
        status,
        percent,
        phase,
    } = event
    else {
        return;
    };
    on_progress(PluginBuildProgress {
        plugin_id,
        status,
        percent,
        phase,
    });
}

fn to_core_result(result: &BuildResult) -> CoreBuildResult {
    CoreBuildResult {
        plugin_id: result.plugin_id.clone(),
        success: result.success,
        output: result.output.clone(),
        skipped: result.skipped,
    }
}
