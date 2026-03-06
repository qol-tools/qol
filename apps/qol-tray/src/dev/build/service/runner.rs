use std::collections::HashMap;
use std::path::PathBuf;

use crate::dev::adapters::traits::CargoPluginBuilder;
use crate::dev::core::{self, BuildStatus};

use super::super::planning::{classify_plan, queued_plugins, PlanDisposition, SkipRecord};
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

    fn run(mut self) -> BuildRun {
        self.events.run_started();
        self.emit_queued();
        let plans = self.plans.clone();
        for plan in &plans {
            self.run_plan(plan);
        }
        self.events.run_finished(&self.results);
        BuildRun {
            plans: self.plans,
            results: self.results,
            fingerprints: self.fingerprints,
        }
    }

    fn emit_queued(&mut self) {
        for queued in queued_plugins(&self.plans) {
            self.events
                .plugin_progress(&queued.plugin_id, BuildStatus::Queued, 0, &queued.phase);
        }
    }

    fn run_plan(&mut self, plan: &PluginBuildPlan) {
        match classify_plan(plan) {
            PlanDisposition::Build => self.build_plan(plan),
            PlanDisposition::Skip(skip) => self.record_skip(plan, skip),
        }
    }

    fn record_skip(&mut self, plan: &PluginBuildPlan, skip: SkipRecord) {
        self.events
            .plugin_progress(&plan.plugin_id, BuildStatus::Skipped, 100, &skip.phase);
        if skip.remove_fingerprint {
            self.fingerprints.remove(&plan.plugin_id);
        }
        self.results.push(BuildResult {
            plugin_id: plan.plugin_id.clone(),
            success: true,
            output: skip.output,
            skipped: true,
        });
    }

    fn build_plan(&mut self, plan: &PluginBuildPlan) {
        self.events.plugin_progress(
            &plan.plugin_id,
            BuildStatus::Building,
            3,
            "Starting cargo build",
        );
        let result = self.run_builder(plan);
        if result.success {
            self.record_success(plan);
        }
        if !result.success {
            self.record_failure(plan);
        }
        self.results.push(result);
    }

    fn run_builder(&mut self, plan: &PluginBuildPlan) -> BuildResult {
        let builder = self.builder;
        let plugin_id = plan.plugin_id.clone();
        let events = &mut self.events;
        let mut progress = |percent, phase: String| {
            events.plugin_progress(&plugin_id, BuildStatus::Building, percent, &phase);
        };
        builder.build_plugin_with_progress(&plugin_id, &plan.path, &mut progress)
    }

    fn record_success(&mut self, plan: &PluginBuildPlan) {
        if let Some(fingerprint) = post_build_fingerprint(plan) {
            self.fingerprints
                .insert(plan.plugin_id.clone(), fingerprint);
        }
        self.events
            .plugin_progress(&plan.plugin_id, BuildStatus::Success, 100, "Build complete");
    }

    fn record_failure(&mut self, plan: &PluginBuildPlan) {
        self.events
            .plugin_progress(&plan.plugin_id, BuildStatus::Failed, 100, "Build failed");
    }
}

fn post_build_fingerprint(plan: &PluginBuildPlan) -> Option<String> {
    super::super::fingerprint::fingerprint_plugin(&plan.path)
        .ok()
        .or_else(|| plan.current_fingerprint.clone())
}
