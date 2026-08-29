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

use platform::{BuildPlatform, Platform};
use qol_workspace::workspace_dev_features;
use std::path::Path;

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
    if let Some(wrapper) = std::env::var_os("QOL_DEV_RUSTC_WRAPPER") {
        command.env("RUSTC_WRAPPER", wrapper);
    }
}

pub fn dev_feature_flags(root: &Path) -> Result<Vec<String>, String> {
    let mut flags = workspace_dev_features(root).map_err(|error| error.to_string())?;
    if root
        .join("apps")
        .join("qol-tray")
        .join("Cargo.toml")
        .is_file()
    {
        for feature in Platform.tray_dev_features().split(',') {
            flags.push(format!("qol-tray/{feature}"));
        }
    }
    Ok(flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_feature_flags_omit_tray_flags_without_tray_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let flags = dev_feature_flags(tmp.path()).unwrap();
        assert!(flags.iter().all(|flag| !flag.starts_with("qol-tray/")));
    }

    #[test]
    fn dev_feature_flags_append_tray_flags_with_tray_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let tray = tmp.path().join("apps").join("qol-tray");
        std::fs::create_dir_all(&tray).unwrap();
        std::fs::write(tray.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        let flags = dev_feature_flags(tmp.path()).unwrap();
        let expected: Vec<String> = Platform
            .tray_dev_features()
            .split(',')
            .map(|feature| format!("qol-tray/{feature}"))
            .collect();
        assert!(flags.ends_with(&expected));
    }
}
