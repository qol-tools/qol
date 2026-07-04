pub mod adapters;
pub mod cargo_build;
pub mod core;
mod fingerprint;
mod fingerprint_store;
pub mod planning;
pub mod registry;
mod service;
pub mod tray;
mod types;

pub use cargo_build::{CargoChild, CargoCommandPluginBuilder};
pub use fingerprint_store::{load_build_fingerprints, save_build_fingerprints};
pub use planning::plan_linked_plugin_builds;
pub use registry::dev_linked_paths;
pub use service::{
    build_linked_plugins, build_linked_plugins_with_core_events,
    build_linked_plugins_with_progress, default_build_application_service, BuildApplicationService,
};
pub use types::{BuildResult, BuildRun, PluginBuildPlan, PluginBuildProgress};
