pub mod bundle;
pub mod desktop;
pub mod desktop_integration;
#[cfg(windows)]
pub mod known_folder;
pub mod shell_execute;
pub mod shell_link;
pub mod start_menu;
pub mod tray_install;

pub use bundle::{
    is_macos_app_bundle, macos_cache_dir, macos_installed_apps, macos_inventory_from_paths,
    macos_launcher_change, macos_launcher_roots, read_macos_app_bundle, read_macos_bundle_facts,
    scan_macos_launcher_root, BundleFacts, InstalledApp, Spotlight, LAUNCHER_ICON_KEY,
};
pub use desktop::{AppEntry, AppRoot};
