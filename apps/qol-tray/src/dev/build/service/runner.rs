mod orchestration;
mod recording;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::dev::adapters::CargoPluginBuilder;
use crate::dev::build::planning::queue::{classify_plan, queued_plugins, PlanDisposition};
use crate::dev::core::{self, BuildStatus};

use super::super::types::{BuildResult, BuildRun, PluginBuildPlan};
use super::events::CoreEventEmitter;

pub(super) struct RunRequest<'a, F>
where
    F: FnMut(core::CoreEvent),
{
    pub(super) dev_links: &'a HashMap<String, PathBuf>,
    pub(super) known_fingerprints: &'a HashMap<String, String>,
    pub(super) builder: &'a dyn CargoPluginBuilder,
    pub(super) on_event: F,
}

pub(super) fn run_build<F>(request: RunRequest<'_, F>) -> BuildRun
where
    F: FnMut(core::CoreEvent),
{
    let plans =
        super::super::plan_linked_plugin_builds(request.dev_links, request.known_fingerprints);
    BuildRunner::new(
        plans,
        request.known_fingerprints,
        request.builder,
        CoreEventEmitter::new(request.on_event),
    )
    .run()
}

struct BuildRunner<'a, F>
where
    F: FnMut(core::CoreEvent),
{
    plans: Vec<PluginBuildPlan>,
    fingerprints: HashMap<String, String>,
    results: Vec<BuildResult>,
    builder: &'a dyn CargoPluginBuilder,
    events: CoreEventEmitter<F>,
}

impl<'a, F> BuildRunner<'a, F>
where
    F: FnMut(core::CoreEvent),
{
    fn new(
        plans: Vec<PluginBuildPlan>,
        known_fingerprints: &HashMap<String, String>,
        builder: &'a dyn CargoPluginBuilder,
        events: CoreEventEmitter<F>,
    ) -> Self {
        Self {
            plans,
            fingerprints: known_fingerprints.clone(),
            results: Vec::new(),
            builder,
            events,
        }
    }

    fn emit_queued(&mut self) {
        for queued in queued_plugins(&self.plans) {
            self.events
                .plugin_progress(&queued.plugin_id, BuildStatus::Queued, 0, &queued.phase);
        }
    }
}

fn classify(plan: &PluginBuildPlan) -> PlanDisposition {
    classify_plan(plan)
}
