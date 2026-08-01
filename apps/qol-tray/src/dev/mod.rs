pub mod adapters;
pub mod boot_contract;
mod config;
mod discovery;
mod linking;
pub mod runtime_gpui;
mod self_build;
#[cfg(feature = "dev")]
pub mod state;

pub use config::DevConfig;
pub use discovery::discover_plugins;
pub use linking::{
    active_dev_links, create_link, get_active_worktree_branch, list_linked_plugins, remove_link,
    LinkRequest, LinkedPlugin,
};
pub use qol_dev_build::core;
pub use qol_dev_build::planning::worktree::{find_git_worktree_base, resolve_worktree_paths};
pub use qol_dev_build::{
    build_linked_plugins_with_core_events, build_linked_plugins_with_progress,
    default_build_application_service, load_build_fingerprints, save_build_fingerprints,
    BuildApplicationService, BuildResult, BuildRun, PluginBuildProgress,
};
pub use runtime_gpui::{normalize_color as normalize_ghost_debug_color, GpuiRuntimeConfig};
pub use self_build::{build_qol_tray_self_with_progress, resolve_qol_tray_self_root};
