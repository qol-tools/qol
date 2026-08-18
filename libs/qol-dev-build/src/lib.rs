pub mod adapters;
pub mod cargo_build;
pub mod core;
mod fingerprint;
mod fingerprint_store;
pub mod freshness;
pub mod planning;
mod platform;
pub mod registry;
pub mod scan_ledger;
mod service;
pub mod target_cache;
pub mod tray;
mod types;

pub use cargo_build::{CargoChild, CargoCommandPluginBuilder};
pub use fingerprint::fingerprint_plugin;
pub use fingerprint_store::{load_build_fingerprints, save_build_fingerprints};
pub use freshness::plugin_binary_exists;
pub use planning::plan_linked_plugin_builds;
pub use registry::dev_linked_paths;
pub use service::{
    build_linked_plugins_with_core_events, build_linked_plugins_with_progress,
    default_build_application_service, linked_plugin_build_timeout, BuildApplicationService,
    MAX_CONCURRENT_PLUGIN_BUILDS,
};
pub use types::{
    BuildResult, BuildRun, PluginBuildPlan, PluginBuildProgress, DEV_BUILD_STATE_FILE,
};

pub fn configure_dev_cargo(command: &mut std::process::Command) {
    let wrapper = std::env::var_os("QOL_DEV_RUSTC_WRAPPER").unwrap_or_default();
    command.env("RUSTC_WRAPPER", wrapper);
}
