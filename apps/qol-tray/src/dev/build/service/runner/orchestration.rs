use crate::dev::build::planning::queue::PlanDisposition;
use crate::dev::build::types::{BuildRun, PluginBuildPlan};
use crate::dev::core;

use super::{classify, BuildRunner};

impl<'a, F> BuildRunner<'a, F>
where
    F: FnMut(core::CoreEvent),
{
    pub(super) fn run(mut self) -> BuildRun {
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

    fn run_plan(&mut self, plan: &PluginBuildPlan) {
        match classify(plan) {
            PlanDisposition::Build => self.build_plan(plan),
            PlanDisposition::Skip(skip) => self.record_skip(plan, skip),
        }
    }
}
