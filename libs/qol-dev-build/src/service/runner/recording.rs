use crate::core::{self, BuildStatus};
use crate::planning::queue::SkipRecord;
use crate::types::PluginBuildPlan;

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
        self.results.push(crate::types::BuildResult {
            plugin_id: plan.plugin_id.clone(),
            success: true,
            output: skip.output,
            skipped: true,
            artifacts: Vec::new(),
        });
    }
}
