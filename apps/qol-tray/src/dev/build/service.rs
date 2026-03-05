mod runner;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::dev::adapters::traits::{BuildFingerprintStore, CargoPluginBuilder, CoreEventSink};
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

    pub fn run(&self, dev_links: &HashMap<String, PathBuf>, config_dir: Option<&Path>) -> BuildRun {
        let known_fingerprints = config_dir
            .map(|dir| self.fingerprint_store.load(dir))
            .unwrap_or_default();

        let build_run = runner::run_build(runner::RunRequest {
            dev_links,
            known_fingerprints: &known_fingerprints,
            builder: self.builder,
            on_event: |event| self.event_sink.publish(event),
        });

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
