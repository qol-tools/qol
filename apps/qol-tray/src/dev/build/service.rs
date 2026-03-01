use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::dev::adapters::traits::{BuildFingerprintStore, CargoPluginBuilder, CoreEventSink};
use crate::dev::core::{self, BuildStatus, CoreBuildResult, CoreInput};

use super::cargo_build::CargoCommandPluginBuilder;
use super::fingerprint_store::JSON_BUILD_FINGERPRINT_STORE;
use super::types::{BuildResult, BuildRun, PluginBuildProgress};
use super::plan_linked_plugin_builds;

pub struct BuildApplicationService<'a> {
    builder: &'a dyn CargoPluginBuilder,
    fingerprint_store: &'a dyn BuildFingerprintStore,
    event_sink: &'a dyn CoreEventSink,
}

impl<'a> BuildApplicationService<'a> {
    pub fn new(
        builder: &'a dyn CargoPluginBuilder,
        fingerprint_store: &'a dyn BuildFingerprintStore,
        event_sink: &'a dyn CoreEventSink,
    ) -> Self {
        Self {
            builder,
            fingerprint_store,
            event_sink,
        }
    }

    pub fn run(
        &self,
        dev_links: &HashMap<String, PathBuf>,
        config_dir: Option<&Path>,
    ) -> BuildRun {
        let known_fingerprints = config_dir
            .map(|dir| self.fingerprint_store.load(dir))
            .unwrap_or_default();

        let build_run = build_linked_plugins_with_core_events_and_builder(
            dev_links,
            &known_fingerprints,
            self.builder,
            |event| self.event_sink.publish(event),
        );

        if let Some(dir) = config_dir {
            if let Err(error) = self.fingerprint_store.save(dir, &build_run.fingerprints) {
                log::error!("Failed to persist build fingerprints: {}", error);
            }
        }

        build_run
    }
}

pub fn default_build_application_service(
    event_sink: &dyn CoreEventSink,
) -> BuildApplicationService<'_> {
    BuildApplicationService::new(
        &CargoCommandPluginBuilder,
        &JSON_BUILD_FINGERPRINT_STORE,
        event_sink,
    )
}

fn to_core_result(result: &BuildResult) -> CoreBuildResult {
    CoreBuildResult {
        plugin_id: result.plugin_id.clone(),
        success: result.success,
        output: result.output.clone(),
        skipped: result.skipped,
    }
}

fn emit_core_input<F>(state: &mut core::CoreState, input: CoreInput, on_event: &mut F)
where
    F: FnMut(core::CoreEvent),
{
    let (next_state, events) = core::reduce(std::mem::take(state), input);
    *state = next_state;
    for event in events {
        on_event(event);
    }
}

fn emit_plugin_progress<F>(
    state: &mut core::CoreState,
    on_event: &mut F,
    plugin_id: &str,
    status: BuildStatus,
    percent: u8,
    phase: &str,
) where
    F: FnMut(core::CoreEvent),
{
    emit_core_input(
        state,
        CoreInput::PluginProgress {
            plugin_id: plugin_id.to_string(),
            status,
            percent,
            phase: phase.to_string(),
        },
        on_event,
    );
}

pub fn build_linked_plugins_with_core_events<F>(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
    on_event: F,
) -> BuildRun
where
    F: FnMut(core::CoreEvent),
{
    build_linked_plugins_with_core_events_and_builder(
        dev_links,
        known_fingerprints,
        &CargoCommandPluginBuilder,
        on_event,
    )
}

fn build_linked_plugins_with_core_events_and_builder<F>(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
    builder: &dyn CargoPluginBuilder,
    mut on_event: F,
) -> BuildRun
where
    F: FnMut(core::CoreEvent),
{
    let plans = plan_linked_plugin_builds(dev_links, known_fingerprints);
    let mut fingerprints = known_fingerprints.clone();
    let mut results = Vec::new();
    let mut core_state = core::CoreState::default();

    emit_core_input(&mut core_state, CoreInput::RunStarted, &mut on_event);

    for plan in &plans {
        if !(plan.has_cargo && plan.supports_platform && plan.needs_rebuild) {
            continue;
        }
        emit_plugin_progress(
            &mut core_state, &mut on_event,
            &plan.plugin_id, BuildStatus::Queued, 0, &plan.reason,
        );
    }

    for plan in &plans {
        if !plan.has_cargo {
            emit_plugin_progress(
                &mut core_state, &mut on_event,
                &plan.plugin_id, BuildStatus::Skipped, 100, "Skipped: Cargo.toml missing",
            );
            fingerprints.remove(&plan.plugin_id);
            results.push(BuildResult {
                plugin_id: plan.plugin_id.clone(),
                success: true,
                output: "Skipped: Cargo.toml missing".to_string(),
                skipped: true,
            });
            continue;
        }

        if !plan.supports_platform {
            emit_plugin_progress(
                &mut core_state, &mut on_event,
                &plan.plugin_id, BuildStatus::Skipped, 100, &plan.reason,
            );
            results.push(BuildResult {
                plugin_id: plan.plugin_id.clone(),
                success: true,
                output: plan.reason.clone(),
                skipped: true,
            });
            continue;
        }

        if !plan.needs_rebuild {
            emit_plugin_progress(
                &mut core_state, &mut on_event,
                &plan.plugin_id, BuildStatus::Skipped, 100, "Up to date",
            );
            results.push(BuildResult {
                plugin_id: plan.plugin_id.clone(),
                success: true,
                output: "Skipped: Up to date".to_string(),
                skipped: true,
            });
            continue;
        }

        emit_plugin_progress(
            &mut core_state, &mut on_event,
            &plan.plugin_id, BuildStatus::Building, 3, "Starting cargo build",
        );

        let mut progress = |percent, phase: String| {
            emit_plugin_progress(
                &mut core_state, &mut on_event,
                &plan.plugin_id, BuildStatus::Building, percent, &phase,
            );
        };
        let result = builder.build_plugin_with_progress(&plan.plugin_id, &plan.path, &mut progress);

        if result.success {
            let post_build_fingerprint = super::fingerprint::fingerprint_plugin(&plan.path)
                .ok()
                .or_else(|| plan.current_fingerprint.clone());
            if let Some(fp) = post_build_fingerprint {
                fingerprints.insert(plan.plugin_id.clone(), fp);
            }
            emit_plugin_progress(
                &mut core_state, &mut on_event,
                &plan.plugin_id, BuildStatus::Success, 100, "Build complete",
            );
        } else {
            emit_plugin_progress(
                &mut core_state, &mut on_event,
                &plan.plugin_id, BuildStatus::Failed, 100, "Build failed",
            );
        }

        results.push(result);
    }

    emit_core_input(
        &mut core_state,
        CoreInput::RunFinished {
            results: results.iter().map(to_core_result).collect(),
        },
        &mut on_event,
    );

    BuildRun {
        plans,
        results,
        fingerprints,
    }
}

pub fn build_linked_plugins_with_progress<F>(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
    mut on_progress: F,
) -> BuildRun
where
    F: FnMut(PluginBuildProgress),
{
    build_linked_plugins_with_core_events(dev_links, known_fingerprints, |event| {
        if let core::CoreEvent::BuildPluginProgress {
            plugin_id,
            status,
            percent,
            phase,
        } = event
        {
            on_progress(PluginBuildProgress {
                plugin_id,
                status,
                percent,
                phase,
            });
        }
    })
}

pub fn build_linked_plugins(dev_links: &HashMap<String, PathBuf>) -> Vec<BuildResult> {
    build_linked_plugins_with_progress(dev_links, &HashMap::new(), |_| {}).results
}
