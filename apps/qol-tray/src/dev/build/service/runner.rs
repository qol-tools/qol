use std::collections::HashMap;
use std::path::PathBuf;

use crate::dev::adapters::traits::CargoPluginBuilder;
use crate::dev::core::{self, BuildStatus, CoreBuildResult, CoreInput, CoreState};

use super::super::types::{BuildResult, BuildRun, PluginBuildPlan};

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
        request.on_event,
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
    core_state: CoreState,
    on_event: F,
}

impl<'a, F> BuildRunner<'a, F>
where
    F: FnMut(core::CoreEvent),
{
    fn new(
        plans: Vec<PluginBuildPlan>,
        known_fingerprints: &HashMap<String, String>,
        builder: &'a dyn CargoPluginBuilder,
        on_event: F,
    ) -> Self {
        Self {
            plans,
            fingerprints: known_fingerprints.clone(),
            results: Vec::new(),
            builder,
            core_state: CoreState::default(),
            on_event,
        }
    }

    fn run(mut self) -> BuildRun {
        self.start();
        self.queue_buildable();
        let plans = self.plans.clone();
        for plan in &plans {
            self.run_plan(plan);
        }
        self.finish();
        BuildRun {
            plans: self.plans,
            results: self.results,
            fingerprints: self.fingerprints,
        }
    }

    fn start(&mut self) {
        self.emit_input(CoreInput::RunStarted);
    }

    fn queue_buildable(&mut self) {
        let queued: Vec<_> = self
            .plans
            .iter()
            .filter(|plan| buildable(plan))
            .map(|plan| (plan.plugin_id.clone(), plan.reason.clone()))
            .collect();
        for (plugin_id, reason) in queued {
            self.emit_progress(&plugin_id, BuildStatus::Queued, 0, &reason);
        }
    }

    fn run_plan(&mut self, plan: &PluginBuildPlan) {
        if !plan.has_cargo {
            self.record_skip(plan, missing_cargo_skip());
            return;
        }
        if !plan.supports_platform {
            self.record_skip(plan, unsupported_platform_skip(plan));
            return;
        }
        if !plan.needs_rebuild {
            self.record_skip(plan, up_to_date_skip());
            return;
        }
        self.build_plan(plan);
    }

    fn record_skip(&mut self, plan: &PluginBuildPlan, skip: SkipRecord) {
        self.emit_progress(&plan.plugin_id, BuildStatus::Skipped, 100, &skip.phase);
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
        self.emit_progress(
            &plan.plugin_id,
            BuildStatus::Building,
            3,
            "Starting cargo build",
        );
        let result = self.run_builder(plan);
        if result.success {
            self.record_success(plan);
        } else {
            self.record_failure(plan);
        }
        self.results.push(result);
    }

    fn run_builder(&mut self, plan: &PluginBuildPlan) -> BuildResult {
        let builder = self.builder;
        let plugin_id = plan.plugin_id.clone();
        let mut progress = |percent, phase: String| {
            self.emit_progress(&plugin_id, BuildStatus::Building, percent, &phase);
        };
        builder.build_plugin_with_progress(&plugin_id, &plan.path, &mut progress)
    }

    fn record_success(&mut self, plan: &PluginBuildPlan) {
        if let Some(fingerprint) = post_build_fingerprint(plan) {
            self.fingerprints
                .insert(plan.plugin_id.clone(), fingerprint);
        }
        self.emit_progress(&plan.plugin_id, BuildStatus::Success, 100, "Build complete");
    }

    fn record_failure(&mut self, plan: &PluginBuildPlan) {
        self.emit_progress(&plan.plugin_id, BuildStatus::Failed, 100, "Build failed");
    }

    fn finish(&mut self) {
        self.emit_input(CoreInput::RunFinished {
            results: self.results.iter().map(to_core_result).collect(),
        });
    }

    fn emit_progress(&mut self, plugin_id: &str, status: BuildStatus, percent: u8, phase: &str) {
        self.emit_input(CoreInput::PluginProgress {
            plugin_id: plugin_id.to_string(),
            status,
            percent,
            phase: phase.to_string(),
        });
    }

    fn emit_input(&mut self, input: CoreInput) {
        let (next_state, events) = core::reduce(std::mem::take(&mut self.core_state), input);
        self.core_state = next_state;
        for event in events {
            (self.on_event)(event);
        }
    }
}

struct SkipRecord {
    phase: String,
    output: String,
    remove_fingerprint: bool,
}

fn buildable(plan: &PluginBuildPlan) -> bool {
    plan.has_cargo && plan.supports_platform && plan.needs_rebuild
}

fn missing_cargo_skip() -> SkipRecord {
    SkipRecord {
        phase: "Skipped: Cargo.toml missing".to_string(),
        output: "Skipped: Cargo.toml missing".to_string(),
        remove_fingerprint: true,
    }
}

fn unsupported_platform_skip(plan: &PluginBuildPlan) -> SkipRecord {
    SkipRecord {
        phase: plan.reason.clone(),
        output: plan.reason.clone(),
        remove_fingerprint: false,
    }
}

fn up_to_date_skip() -> SkipRecord {
    SkipRecord {
        phase: "Up to date".to_string(),
        output: "Skipped: Up to date".to_string(),
        remove_fingerprint: false,
    }
}

fn post_build_fingerprint(plan: &PluginBuildPlan) -> Option<String> {
    super::super::fingerprint::fingerprint_plugin(&plan.path)
        .ok()
        .or_else(|| plan.current_fingerprint.clone())
}

fn to_core_result(result: &BuildResult) -> CoreBuildResult {
    CoreBuildResult {
        plugin_id: result.plugin_id.clone(),
        success: result.success,
        output: result.output.clone(),
        skipped: result.skipped,
    }
}
