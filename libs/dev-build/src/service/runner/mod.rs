mod orchestration;
mod recording;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::adapters::CargoPluginBuilder;
use crate::core::{self, BuildStatus};
use crate::planning::queue::{classify_plan, queued_plugins, PlanDisposition};

use super::super::types::{BuildResult, BuildRun, PluginBuildPlan};
use super::events::CoreEventEmitter;

pub(super) struct RunRequest<'a, F>
where
    F: FnMut(core::CoreEvent),
{
    pub(super) dev_links: &'a HashMap<String, PathBuf>,
    pub(super) builder: &'a dyn CargoPluginBuilder,
    pub(super) worktree_branch: Option<&'a str>,
    pub(super) on_event: F,
}

pub(super) fn run_build<F>(request: RunRequest<'_, F>) -> BuildRun
where
    F: FnMut(core::CoreEvent),
{
    let plans = super::super::plan_linked_plugin_builds(request.dev_links, request.worktree_branch);
    BuildRunner::new(
        plans,
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
        builder: &'a dyn CargoPluginBuilder,
        events: CoreEventEmitter<F>,
    ) -> Self {
        Self {
            plans,
            fingerprints: HashMap::new(),
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
