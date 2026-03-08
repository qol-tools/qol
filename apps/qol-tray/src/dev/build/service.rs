mod events;
mod persistence;
mod runner;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::dev::adapters::{BuildFingerprintStore, CargoPluginBuilder, CoreEventSink};
use crate::dev::core;

use super::cargo_build::CargoCommandPluginBuilder;
use super::fingerprint_store::JSON_BUILD_FINGERPRINT_STORE;
use super::types::{BuildResult, BuildRun, PluginBuildProgress};

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
        worktree_branch: Option<&str>,
    ) -> BuildRun {
        let known_fingerprints =
            persistence::load_known_fingerprints(self.fingerprint_store, config_dir);
        let build_run = runner::run_build(runner::RunRequest {
            dev_links,
            known_fingerprints: &known_fingerprints,
            builder: self.builder,
            worktree_branch,
            on_event: |event| self.event_sink.publish(event),
        });
        persistence::persist_build_run(self.fingerprint_store, config_dir, &build_run);
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

pub fn build_linked_plugins_with_core_events<F>(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
    on_event: F,
) -> BuildRun
where
    F: FnMut(core::CoreEvent),
{
    runner::run_build(runner::RunRequest {
        dev_links,
        known_fingerprints,
        builder: &CargoCommandPluginBuilder,
        worktree_branch: None,
        on_event,
    })
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
        events::emit_plugin_progress(event, &mut on_progress);
    })
}

pub fn build_linked_plugins(dev_links: &HashMap<String, PathBuf>) -> Vec<BuildResult> {
    build_linked_plugins_with_progress(dev_links, &HashMap::new(), |_| {}).results
}
