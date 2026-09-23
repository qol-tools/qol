mod events;
mod runner;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::adapters::{CargoPluginBuilder, CoreEventSink};
use crate::core;

use super::cargo_build::CargoCommandPluginBuilder;
use super::types::{BuildRun, PluginBuildProgress};

pub const MAX_CONCURRENT_PLUGIN_BUILDS: usize = 4;

const BUILD_RUN_TIMEOUT_MARGIN: Duration = Duration::from_secs(10);

pub fn linked_plugin_build_timeout(plugin_count: usize) -> Duration {
    let waves = plugin_count.saturating_add(MAX_CONCURRENT_PLUGIN_BUILDS - 1)
        / MAX_CONCURRENT_PLUGIN_BUILDS;
    let waves = waves.max(1);
    let per_wave = crate::cargo_build::BUILD_TIMEOUT
        .saturating_add(crate::cargo_build::BUILD_TERMINATION_GRACE);
    per_wave
        .saturating_mul(u32::try_from(waves).unwrap_or(u32::MAX))
        .saturating_add(BUILD_RUN_TIMEOUT_MARGIN)
}

pub struct BuildApplicationService<'a> {
    builder: &'a dyn CargoPluginBuilder,
    event_sink: &'a dyn CoreEventSink,
}

impl<'a> BuildApplicationService<'a> {
    pub fn new(builder: &'a dyn CargoPluginBuilder, event_sink: &'a dyn CoreEventSink) -> Self {
        Self {
            builder,
            event_sink,
        }
    }

    pub fn run(
        &self,
        dev_links: &HashMap<String, PathBuf>,
        _config_dir: Option<&Path>,
        worktree_branch: Option<&str>,
    ) -> BuildRun {
        runner::run_build(runner::RunRequest {
            dev_links,
            builder: self.builder,
            worktree_branch,
            on_event: |event| self.event_sink.publish(event),
        })
    }
}

pub fn default_build_application_service(
    event_sink: &dyn CoreEventSink,
) -> BuildApplicationService<'_> {
    BuildApplicationService::new(&CargoCommandPluginBuilder, event_sink)
}

pub fn build_linked_plugins_with_core_events<F>(
    dev_links: &HashMap<String, PathBuf>,
    on_event: F,
) -> BuildRun
where
    F: FnMut(core::CoreEvent),
{
    runner::run_build(runner::RunRequest {
        dev_links,
        builder: &CargoCommandPluginBuilder,
        worktree_branch: None,
        on_event,
    })
}

pub fn build_linked_plugins_with_progress<F>(
    dev_links: &HashMap<String, PathBuf>,
    mut on_progress: F,
) -> BuildRun
where
    F: FnMut(PluginBuildProgress),
{
    build_linked_plugins_with_core_events(dev_links, |event| {
        events::emit_plugin_progress(event, &mut on_progress);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_plugin_timeout_covers_queued_build_waves() {
        let one_wave = linked_plugin_build_timeout(1);
        assert_eq!(
            one_wave,
            linked_plugin_build_timeout(MAX_CONCURRENT_PLUGIN_BUILDS)
        );
        assert!(linked_plugin_build_timeout(MAX_CONCURRENT_PLUGIN_BUILDS + 1) > one_wave);
        assert_eq!(
            linked_plugin_build_timeout(MAX_CONCURRENT_PLUGIN_BUILDS * 2),
            linked_plugin_build_timeout(MAX_CONCURRENT_PLUGIN_BUILDS + 1)
        );
        assert_eq!(
            linked_plugin_build_timeout(0),
            one_wave,
            "an empty or unavailable link snapshot still gets a complete first-wave budget"
        );
    }
}
