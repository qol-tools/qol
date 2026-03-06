use std::collections::HashMap;

use crate::dev::build::planning::queue::SkipRecord;
use crate::dev::build::types::{BuildResult, PluginBuildPlan};
use crate::dev::core::{self, BuildStatus};

use super::BuildRunner;

impl<'a, F> BuildRunner<'a, F>
where
    F: FnMut(core::CoreEvent),
{
    pub(super) fn record_skip(&mut self, plan: &PluginBuildPlan, skip: SkipRecord) {
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

    pub(super) fn record_built_result(&mut self, plan: &PluginBuildPlan, result: BuildResult) {
        if result.success {
            self.record_success(plan);
        } else {
            self.record_failure(plan);
        }
        self.results.push(result);
    }

    fn record_success(&mut self, plan: &PluginBuildPlan) {
        update_post_build_fingerprint(&mut self.fingerprints, plan);
        self.events
            .plugin_progress(&plan.plugin_id, BuildStatus::Success, 100, "Build complete");
    }

    fn record_failure(&mut self, plan: &PluginBuildPlan) {
        self.events
            .plugin_progress(&plan.plugin_id, BuildStatus::Failed, 100, "Build failed");
    }
}

fn update_post_build_fingerprint(
    fingerprints: &mut HashMap<String, String>,
    plan: &PluginBuildPlan,
) {
    if let Some(fingerprint) = post_build_fingerprint(plan) {
        fingerprints.insert(plan.plugin_id.clone(), fingerprint);
    }
}

fn post_build_fingerprint(plan: &PluginBuildPlan) -> Option<String> {
    crate::dev::build::fingerprint::fingerprint_plugin(&plan.path)
        .ok()
        .or_else(|| plan.current_fingerprint.clone())
}
