use std::sync::mpsc;
use std::sync::Mutex;

use crate::dev::build::planning::queue::PlanDisposition;
use crate::dev::build::types::{BuildResult, BuildRun, PluginBuildPlan};
use crate::dev::core::{self, BuildStatus};

use super::{classify, BuildRunner};

enum BuildMessage {
    Progress {
        plugin_id: String,
        percent: u8,
        phase: String,
    },
    Done {
        plan_index: usize,
        result: BuildResult,
    },
}

impl<'a, F> BuildRunner<'a, F>
where
    F: FnMut(core::CoreEvent),
{
    pub(super) fn run(mut self) -> BuildRun {
        self.events.run_started();
        self.emit_queued();

        let (builds, skips) = partition_plans(&self.plans);

        for (i, disposition) in skips {
            if let PlanDisposition::Skip(skip) = disposition {
                let plan = self.plans[i].clone();
                self.record_skip(&plan, skip);
            }
        }

        if !builds.is_empty() {
            self.run_builds_parallel(&builds);
        }

        self.events.run_finished(&self.results);
        BuildRun {
            plans: self.plans,
            results: self.results,
            fingerprints: self.fingerprints,
        }
    }

    fn run_builds_parallel(&mut self, build_indices: &[usize]) {
        let (tx, rx) = mpsc::channel::<BuildMessage>();
        let completed: Mutex<Vec<(usize, BuildResult)>> = Mutex::new(Vec::new());
        let builder = self.builder;
        let plans = &self.plans;

        std::thread::scope(|scope| {
            for &idx in build_indices {
                let tx = tx.clone();
                let plan = &plans[idx];
                scope.spawn(move || {
                    let id = plan.plugin_id.clone();
                    let mut on_progress = |percent: u8, phase: String| {
                        let _ = tx.send(BuildMessage::Progress {
                            plugin_id: id.clone(),
                            percent,
                            phase,
                        });
                    };
                    let result = builder.build_plugin_with_progress(
                        &plan.plugin_id,
                        &plan.path,
                        &mut on_progress,
                    );
                    let _ = tx.send(BuildMessage::Done {
                        plan_index: idx,
                        result,
                    });
                });
            }
            drop(tx);

            for msg in rx {
                match msg {
                    BuildMessage::Progress {
                        plugin_id,
                        percent,
                        phase,
                    } => {
                        self.events.plugin_progress(
                            &plugin_id,
                            BuildStatus::Building,
                            percent,
                            &phase,
                        );
                    }
                    BuildMessage::Done { plan_index, result } => {
                        completed.lock().unwrap().push((plan_index, result));
                    }
                }
            }
        });

        let mut done = completed.into_inner().unwrap();
        done.sort_by_key(|(i, _)| *i);
        for (idx, result) in done {
            self.record_build_result(&self.plans[idx].clone(), result);
        }
    }

    fn record_build_result(&mut self, plan: &PluginBuildPlan, result: BuildResult) {
        if result.success {
            update_fingerprint(&mut self.fingerprints, plan);
            self.events.plugin_progress(
                &plan.plugin_id,
                BuildStatus::Success,
                100,
                "Build complete",
            );
        } else {
            self.events
                .plugin_progress(&plan.plugin_id, BuildStatus::Failed, 100, "Build failed");
        }
        self.results.push(result);
    }
}

fn partition_plans(plans: &[PluginBuildPlan]) -> (Vec<usize>, Vec<(usize, PlanDisposition)>) {
    let mut builds = Vec::new();
    let mut skips = Vec::new();
    for (i, plan) in plans.iter().enumerate() {
        match classify(plan) {
            PlanDisposition::Build => builds.push(i),
            skip @ PlanDisposition::Skip(_) => skips.push((i, skip)),
        }
    }
    (builds, skips)
}

fn update_fingerprint(
    fingerprints: &mut std::collections::HashMap<String, String>,
    plan: &PluginBuildPlan,
) {
    if let Some(fp) = crate::dev::build::fingerprint::fingerprint_plugin(&plan.path)
        .ok()
        .or_else(|| plan.current_fingerprint.clone())
    {
        fingerprints.insert(plan.plugin_id.clone(), fp);
    }
}
