pub mod adapters;
mod build;
mod config;
pub mod core;
mod discovery;
mod linking;
#[cfg(feature = "dev")]
pub mod state;

pub use build::build_linked_plugins;
pub use build::build_linked_plugins_with_core_events;
pub use build::build_linked_plugins_with_progress;
pub use build::build_qol_tray_self_with_progress;
pub use build::default_build_application_service;
pub use build::load_build_fingerprints;
pub use build::save_build_fingerprints;
pub use build::BuildApplicationService;
pub use build::BuildResult;
pub use build::BuildRun;
pub use build::PluginBuildProgress;
pub use config::DevConfig;
pub use discovery::discover_plugins;
pub use linking::{
    create_link, list_linked_plugins, load_dev_links, remove_link, LinkRequest, LinkedPlugin,
};
