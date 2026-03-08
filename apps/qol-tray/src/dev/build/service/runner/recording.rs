use crate::dev::build::planning::queue::SkipRecord;
use crate::dev::build::types::PluginBuildPlan;
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
        self.results.push(crate::dev::build::types::BuildResult {
            plugin_id: plan.plugin_id.clone(),
            success: true,
            output: skip.output,
            skipped: true,
        });
    }
}
