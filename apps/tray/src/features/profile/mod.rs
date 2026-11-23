pub mod core;
pub(crate) mod http;
pub(crate) mod pull_request_builds;
pub mod registry;
pub mod scope_store;
mod startup;
pub mod sync;

use anyhow::Context;

pub use scope_store::{ProfileScopeStore, ScopeKind};
pub(crate) use startup::run_startup_cleanup;

pub(crate) fn update_settings_file(
    file: &qol_config::profile::SettingsFile,
    update: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) -> anyhow::Result<()> {
    let config_dir = crate::paths::shared_config_dir()?;
    let _guard = crate::plugins::config::profile_config_write_guard_unmarked();
    file.update(&config_dir, update)
        .with_context(|| format!("failed to save {}", file.name()))
}

pub(crate) fn check_settings_file(file: &qol_config::profile::SettingsFile) -> anyhow::Result<()> {
    let config_dir = crate::paths::shared_config_dir()?;
    file.check_updatable(&config_dir)
        .with_context(|| format!("failed to read {}", file.name()))
}

pub(crate) fn feature_flag_enabled(flag: &qol_config::feature_flags::FeatureFlag) -> bool {
    crate::paths::profile_dir().is_ok_and(|root| {
        let core = qol_plugin_api::profile::active_core_dir(&root);
        qol_config::feature_flags::enabled_in(&core, flag)
    })
}
