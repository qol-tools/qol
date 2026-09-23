pub mod bundle;
pub mod desktop;
pub mod desktop_integration;

pub use bundle::{
    is_macos_app_bundle, macos_cache_dir, macos_installed_apps, macos_inventory_from_paths,
    macos_launcher_roots, read_macos_app_bundle, scan_macos_launcher_root, InstalledApp, Spotlight,
};
pub use desktop::{AppEntry, AppRoot};
