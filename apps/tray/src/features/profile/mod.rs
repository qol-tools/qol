pub mod core;
pub(crate) mod http;
pub(crate) mod pull_request_builds;
pub mod registry;
pub mod scope_store;
mod startup;
pub mod sync;

pub use scope_store::{ProfileScopeStore, ScopeKind};
pub(crate) use startup::run_startup_cleanup;

pub(crate) fn feature_flag_enabled(flag: &qol_config::feature_flags::FeatureFlag) -> bool {
    crate::paths::profile_dir().is_ok_and(|root| {
        let core = qol_plugin_api::profile::active_core_dir(&root);
        qol_config::feature_flags::enabled_in(&core, flag)
    })
}
