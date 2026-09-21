pub mod capability;
pub mod host_exec;
pub mod launcher_flows;
pub mod manifest;
pub mod permissions;
pub mod restore;

pub use manifest::PluginId;
pub use restore::{ForegroundProc, PaneSnapshot, RestoreClaim};

/// The expanding crate's own `plugin.toml`, embedded at compile time.
#[macro_export]
macro_rules! plugin_manifest_source {
    () => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/plugin.toml"))
    };
}

/// The `permissions` doctor check, built from the expanding crate's manifest.
#[macro_export]
macro_rules! permissions_check {
    ($plugin_name:expr) => {
        $crate::permissions::permissions_check($plugin_name, $crate::plugin_manifest_source!())
    };
}
