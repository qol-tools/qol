pub mod adapters;
pub mod boot_contract;
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
pub use build::planning::worktree::{find_git_worktree_base, resolve_worktree_paths};
pub use build::resolve_qol_tray_self_root;
pub use build::save_build_fingerprints;
pub use build::BuildApplicationService;
pub use build::BuildResult;
pub use build::BuildRun;
pub use build::PluginBuildProgress;
pub use config::DevConfig;
pub use discovery::discover_plugins;
pub use linking::{
    active_dev_links, create_link, get_active_worktree_branch, list_linked_plugins, remove_link,
    LinkRequest, LinkedPlugin,
};
