use crate::dev::build::types::{BuildResult, PluginBuildPlan};
use crate::dev::core::{self, BuildStatus};

use super::BuildRunner;

impl<'a, F> BuildRunner<'a, F>
where
    F: FnMut(core::CoreEvent),
{
    pub(super) fn build_plan(&mut self, plan: &PluginBuildPlan) {
        let result = self.execute_build(plan);
        self.record_built_result(plan, result);
    }

    fn execute_build(&mut self, plan: &PluginBuildPlan) -> BuildResult {
        self.events.plugin_progress(
            &plan.plugin_id,
            BuildStatus::Building,
            3,
            "Starting cargo build",
        );
        let builder = self.builder;
        let plugin_id = plan.plugin_id.clone();
        let events = &mut self.events;
        let mut progress = |percent, phase: String| {
            events.plugin_progress(&plugin_id, BuildStatus::Building, percent, &phase);
        };
        builder.build_plugin_with_progress(&plugin_id, &plan.path, &mut progress)
    }
}
