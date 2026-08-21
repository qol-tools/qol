pub mod adapters;
pub mod cargo_build;
pub mod core;
mod fingerprint;
pub mod freshness;
pub mod planning;
mod platform;
pub mod registry;
pub mod scan_ledger;
mod service;
mod sidecar;
pub mod target_cache;
pub mod tray;
mod types;

pub use cargo_build::{CargoChild, CargoCommandPluginBuilder};
pub use fingerprint::fingerprint_plugin;
pub use freshness::{plugin_binary_exists, plugin_binary_path};
pub use planning::plan_linked_plugin_builds;
pub use registry::dev_linked_paths;
pub use service::{
    build_linked_plugins_with_core_events, build_linked_plugins_with_progress,
    default_build_application_service, linked_plugin_build_timeout, BuildApplicationService,
    MAX_CONCURRENT_PLUGIN_BUILDS,
};
pub use sidecar::{
    binary_is_fresh, daemons_needing_restart, fingerprint_sidecar_path, read_fingerprint_sidecar,
    write_fingerprint_sidecar,
};
pub use types::{BuildResult, BuildRun, PluginBuildPlan, PluginBuildProgress};

pub fn configure_dev_cargo(command: &mut std::process::Command) {
    let wrapper = std::env::var_os("QOL_DEV_RUSTC_WRAPPER").unwrap_or_default();
    command.env("RUSTC_WRAPPER", wrapper);
}
