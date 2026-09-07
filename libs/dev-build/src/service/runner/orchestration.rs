use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use crate::core::{self, BuildStatus};
use crate::planning::queue::PlanDisposition;
use crate::types::{BuildResult, BuildRun, PluginBuildPlan};

use super::super::MAX_CONCURRENT_PLUGIN_BUILDS;
use super::{classify, BuildRunner};

struct BuildJob {
    plan_index: usize,
    queued_at: Instant,
}

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
            self.run_builds(&builds);
        }

        self.events.run_finished(&self.results);
        BuildRun {
            plans: self.plans,
            results: self.results,
            fingerprints: self.fingerprints,
        }
    }

    fn run_builds_parallel(&mut self, build_indices: &[usize]) {
        let (job_tx, job_rx) = mpsc::channel::<BuildJob>();
        let (message_tx, message_rx) = mpsc::channel::<BuildMessage>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let active = Arc::new(AtomicUsize::new(0));
        let run_started = Instant::now();
        let worker_count = build_indices.len().min(MAX_CONCURRENT_PLUGIN_BUILDS);
        log::debug!(
            "[dev-build] event=queue plugin_count={} worker_limit={} elapsed_ms=0",
            build_indices.len(),
            worker_count
        );

        let builder = self.builder;
        let plans = &self.plans;
        let mut completed = Vec::with_capacity(build_indices.len());

        std::thread::scope(|scope| {
            for _ in 0..worker_count {
                let job_rx = Arc::clone(&job_rx);
                let tx = message_tx.clone();
                let active = Arc::clone(&active);
                scope.spawn(move || {
                    loop {
                        let job = match job_rx.lock() {
                            Ok(receiver) => receiver.recv().ok(),
                            Err(_) => None,
                        };
                        let Some(job) = job else {
                            return;
                        };
                        let plan = &plans[job.plan_index];
                        let plugin_id = plan.plugin_id.clone();
                        let active_count = active.fetch_add(1, Ordering::SeqCst) + 1;
                        log::debug!(
                            "[dev-build] event=admit plugin_id={} active={} queue_wait_ms={} elapsed_ms={}",
                            plugin_id,
                            active_count,
                            job.queued_at.elapsed().as_millis(),
                            run_started.elapsed().as_millis()
                        );
                        let build_started = Instant::now();
                        let mut on_progress = |percent: u8, phase: String| {
                            let _ = tx.send(BuildMessage::Progress {
                                plugin_id: plugin_id.clone(),
                                percent,
                                phase,
                            });
                        };
                        let result = builder.build_plugin_with_progress(
                            &plan.plugin_id,
                            &plan.path,
                            &mut on_progress,
                        );
                        let active_count = active.fetch_sub(1, Ordering::SeqCst) - 1;
                        log::debug!(
                            "[dev-build] event=complete plugin_id={} success={} skipped={} completion_reason={} active={} build_elapsed_ms={} elapsed_ms={}",
                            plugin_id,
                            result.success,
                            result.skipped,
                            completion_reason(&result),
                            active_count,
                            build_started.elapsed().as_millis(),
                            run_started.elapsed().as_millis()
                        );
                        let _ = tx.send(BuildMessage::Done {
                            plan_index: job.plan_index,
                            result,
                        });
                    }
                });
            }
            for &plan_index in build_indices {
                if job_tx
                    .send(BuildJob {
                        plan_index,
                        queued_at: Instant::now(),
                    })
                    .is_err()
                {
                    break;
                }
            }
            drop(job_tx);
            drop(message_tx);

            for msg in message_rx {
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
                        completed.push((plan_index, result));
                    }
                }
            }
        });

        completed.sort_by_key(|(i, _)| *i);
        for (idx, result) in completed {
            self.record_build_result(&self.plans[idx].clone(), result);
        }
    }

    fn run_builds(&mut self, build_indices: &[usize]) {
        let plugins: Vec<(&str, &std::path::Path)> = build_indices
            .iter()
            .map(|&plan_index| {
                let plan = &self.plans[plan_index];
                (plan.plugin_id.as_str(), plan.path.as_path())
            })
            .collect();
        let mut on_progress = |plugin_id: &str, percent: u8, phase: String| {
            self.events
                .plugin_progress(plugin_id, BuildStatus::Building, percent, &phase);
        };
        let Some(results) = self
            .builder
            .build_plugins_with_progress(&plugins, &mut on_progress)
        else {
            self.run_builds_parallel(build_indices);
            return;
        };
        if results.len() != build_indices.len() {
            log::error!(
                "[dev-build] event=batch_result_mismatch expected={} actual={}",
                build_indices.len(),
                results.len()
            );
            self.run_builds_parallel(build_indices);
            return;
        }
        for (plan_index, result) in build_indices.iter().copied().zip(results) {
            self.record_build_result(&self.plans[plan_index].clone(), result);
        }
    }

    fn record_build_result(&mut self, plan: &PluginBuildPlan, result: BuildResult) {
        if result.success {
            if let Some(fingerprint) = built_fingerprint(plan) {
                self.fingerprints
                    .insert(plan.plugin_id.clone(), fingerprint.clone());
                persist_fingerprint_sidecar(plan, &fingerprint);
            }
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

fn built_fingerprint(plan: &PluginBuildPlan) -> Option<String> {
    crate::fingerprint::fingerprint_plugin(&plan.path)
        .ok()
        .or_else(|| plan.current_fingerprint.clone())
}

fn persist_fingerprint_sidecar(plan: &PluginBuildPlan, fingerprint: &str) {
    let Some(binary) = crate::freshness::plugin_binary_path(&plan.path) else {
        log::warn!(
            "[dev-build] event=sidecar_skip plugin_id={} reason=no_declared_binary",
            plan.plugin_id
        );
        return;
    };
    if let Err(error) = crate::sidecar::write_fingerprint_sidecar(&binary, fingerprint) {
        log::error!(
            "[dev-build] event=sidecar_write_failed plugin_id={} error={}",
            plan.plugin_id,
            error
        );
    }
}

fn completion_reason(result: &BuildResult) -> &'static str {
    if result.success {
        if result.skipped {
            "skipped"
        } else {
            "success"
        }
    } else {
        "failed"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::CargoPluginBuilder;
    use crate::core::{BuildStatus, CoreEvent};
    use crate::service::events::CoreEventEmitter;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;

    struct TestBuilder {
        active: AtomicUsize,
        maximum: AtomicUsize,
    }

    impl CargoPluginBuilder for TestBuilder {
        fn build_plugin_with_progress(
            &self,
            plugin_id: &str,
            path: &Path,
            on_progress: &mut dyn FnMut(u8, String),
        ) -> BuildResult {
            assert_eq!(path, Path::new("/plugins").join(plugin_id));
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            on_progress(50, "cargo".to_string());
            std::thread::sleep(Duration::from_millis(20));
            self.active.fetch_sub(1, Ordering::SeqCst);

            let (success, output) = match plugin_id {
                "plugin-5" => (false, "compiler error for plugin-5".to_string()),
                _ => (true, format!("built {plugin_id}")),
            };
            BuildResult {
                plugin_id: plugin_id.to_string(),
                success,
                output,
                skipped: false,
                artifacts: Vec::new(),
            }
        }
    }

    struct WorkConservingBuilder {
        starts: std::sync::mpsc::Sender<String>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl CargoPluginBuilder for WorkConservingBuilder {
        fn build_plugin_with_progress(
            &self,
            plugin_id: &str,
            _path: &Path,
            _on_progress: &mut dyn FnMut(u8, String),
        ) -> BuildResult {
            let _ = self.starts.send(plugin_id.to_string());
            if plugin_id == "plugin-0" {
                let (released, condvar) = &*self.release;
                let mut released = released.lock().unwrap();
                while !*released {
                    released = condvar.wait(released).unwrap();
                }
            }
            BuildResult {
                plugin_id: plugin_id.to_string(),
                success: true,
                output: format!("built {plugin_id}"),
                skipped: false,
                artifacts: Vec::new(),
            }
        }
    }

    fn plan(index: usize) -> PluginBuildPlan {
        let plugin_id = format!("plugin-{index}");
        PluginBuildPlan {
            plugin_id: plugin_id.clone(),
            path: PathBuf::from("/plugins").join(&plugin_id),
            has_cargo: true,
            supports_platform: true,
            needs_rebuild: true,
            current_fingerprint: Some(format!("fingerprint-{index}")),
            last_built_fingerprint: None,
            reason: "Source changed".to_string(),
        }
    }

    #[test]
    fn plugin_builds_are_bounded_and_results_keep_mapping() {
        let plans = (0..9).map(plan).collect::<Vec<_>>();
        let build_indices = (0..plans.len()).collect::<Vec<_>>();
        let builder = TestBuilder {
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let mut runner = BuildRunner::new(
            plans,
            &builder,
            CoreEventEmitter::new(move |event| {
                captured_events.lock().unwrap().push(event);
            }),
        );

        runner.run_builds_parallel(&build_indices);

        let maximum = builder.maximum.load(Ordering::SeqCst);
        assert!(maximum > 1, "runner should retain useful parallelism");
        assert!(
            maximum <= MAX_CONCURRENT_PLUGIN_BUILDS,
            "runner exceeded concurrency limit: {maximum}"
        );
        let ids = runner
            .results
            .iter()
            .map(|result| result.plugin_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            (0..9)
                .map(|index| format!("plugin-{index}"))
                .collect::<Vec<_>>()
        );

        let failed = runner
            .results
            .iter()
            .find(|result| result.plugin_id == "plugin-5")
            .expect("failed plugin result");
        assert!(!failed.success);
        assert_eq!(failed.output, "compiler error for plugin-5");

        let events = events.lock().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            CoreEvent::BuildPluginProgress {
                plugin_id,
                status: BuildStatus::Building,
                percent: 50,
                phase,
            } if plugin_id == "plugin-5" && phase == "cargo"
        )));
    }

    #[test]
    fn plugin_builds_admit_next_plugin_when_a_slot_finishes() {
        let plans = (0..5).map(plan).collect::<Vec<_>>();
        let build_indices = (0..plans.len()).collect::<Vec<_>>();
        let (starts_tx, starts_rx) = std::sync::mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let builder = WorkConservingBuilder {
            starts: starts_tx,
            release: Arc::clone(&release),
        };
        let mut runner = BuildRunner::new(plans, &builder, CoreEventEmitter::new(|_| {}));

        std::thread::scope(|scope| {
            let handle = scope.spawn(|| runner.run_builds_parallel(&build_indices));
            let deadline = Instant::now() + Duration::from_millis(500);
            let mut admitted_plugin_4 = false;
            while Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                match starts_rx.recv_timeout(remaining) {
                    Ok(plugin_id) if plugin_id == "plugin-4" => {
                        admitted_plugin_4 = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            let (released, condvar) = &*release;
            *released.lock().unwrap() = true;
            condvar.notify_all();
            assert!(
                admitted_plugin_4,
                "a completed worker must admit the next queued plugin"
            );
            handle.join().unwrap();
        });
    }

    struct SucceedingBuilder;

    impl CargoPluginBuilder for SucceedingBuilder {
        fn build_plugin_with_progress(
            &self,
            plugin_id: &str,
            _path: &Path,
            _on_progress: &mut dyn FnMut(u8, String),
        ) -> BuildResult {
            BuildResult {
                plugin_id: plugin_id.to_string(),
                success: true,
                output: "built".to_string(),
                skipped: false,
                artifacts: Vec::new(),
            }
        }
    }

    fn daemon_plugin_dir(root: &Path) -> PathBuf {
        let plugin_dir = root.join("plugin-a");
        std::fs::create_dir_all(plugin_dir.join("src")).unwrap();
        std::fs::write(
            plugin_dir.join("Cargo.toml"),
            "[package]\nname = \"plugin-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid = \"plugin-a\"\nname = \"A\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"A\"\nitems = []\n\n[daemon]\nenabled = true\ncommand = \"plugin-a\"\n",
        )
        .unwrap();
        plugin_dir
    }

    fn build_plan_for(path: PathBuf) -> PluginBuildPlan {
        PluginBuildPlan {
            plugin_id: "plugin-a".to_string(),
            path,
            has_cargo: true,
            supports_platform: true,
            needs_rebuild: true,
            current_fingerprint: None,
            last_built_fingerprint: None,
            reason: "Source changed".to_string(),
        }
    }

    #[test]
    fn successful_build_writes_fingerprint_sidecar_next_to_binary() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = daemon_plugin_dir(tmp.path());
        let builder = SucceedingBuilder;
        let mut runner = BuildRunner::new(
            vec![build_plan_for(plugin_dir.clone())],
            &builder,
            CoreEventEmitter::new(|_| {}),
        );
        let binary = crate::freshness::plugin_binary_path(&plugin_dir).expect("resolved binary");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, "binary").unwrap();

        runner.run_builds(&[0]);

        let expected = crate::fingerprint_plugin(&plugin_dir).unwrap();
        assert_eq!(
            crate::read_fingerprint_sidecar(&binary).as_deref(),
            Some(expected.as_str()),
            "a successful build must anchor its fingerprint next to the binary"
        );
        assert_eq!(
            runner.fingerprints.get("plugin-a").map(String::as_str),
            Some(expected.as_str())
        );
    }

    #[test]
    fn build_without_declared_binary_writes_no_sidecar() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        std::fs::create_dir_all(plugin_dir.join("src")).unwrap();
        std::fs::write(
            plugin_dir.join("Cargo.toml"),
            "[package]\nname = \"plugin-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid = \"plugin-a\"\nname = \"A\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"A\"\nitems = []\n",
        )
        .unwrap();
        let builder = SucceedingBuilder;
        let mut runner = BuildRunner::new(
            vec![build_plan_for(plugin_dir.clone())],
            &builder,
            CoreEventEmitter::new(|_| {}),
        );

        runner.run_builds(&[0]);

        assert!(crate::freshness::plugin_binary_path(&plugin_dir).is_none());
        assert!(
            !plugin_dir.join("target").exists(),
            "no sidecar may be written without a binary to anchor it"
        );
    }
}
