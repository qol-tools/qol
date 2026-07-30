mod artifact_identity;
mod autostart_target;
#[cfg(feature = "dev")]
mod cargo_target;
#[cfg(feature = "dev")]
mod cargo_target_cache;
#[cfg(feature = "dev")]
mod cargo_target_total;
mod config_parse_failures;
#[cfg(feature = "dev")]
mod dev_link_paths;
#[cfg(feature = "dev")]
mod fingerprint_health;
mod gpu_driver_sync;
mod hotkey_shadows;
mod install_identity;
mod orphan_plugin_configs;
#[cfg(feature = "dev")]
mod plugin_daemon_health;
mod plugin_port_collisions;
mod plugin_process_leaks;
#[cfg(feature = "dev")]
mod plugin_staleness;
mod plugin_uid_table;
#[cfg(feature = "dev")]
mod reserved_plugin_ids;
mod runtime_prereqs;
#[cfg(feature = "dev")]
mod rust_clippy;
#[cfg(feature = "dev")]
mod rust_formatting;
mod shell_hook_present;
#[cfg(feature = "dev")]
mod single_source_guard;

#[cfg(feature = "dev")]
pub(super) use dev_link_paths::relocate_dev_link;
pub use gpu_driver_sync::spawn_watch as spawn_gpu_driver_sync_watch;
#[cfg(feature = "dev")]
pub(super) use plugin_staleness::stale_running_daemons;

use super::framework::DoctorCheck;

pub(super) fn registry() -> Vec<Box<dyn DoctorCheck>> {
    let checks: Vec<Box<dyn DoctorCheck>> = vec![
        Box::new(artifact_identity::ArtifactIdentityCheck),
        Box::new(install_identity::InstallIdentityCheck),
        Box::new(autostart_target::AutostartTargetCheck),
        Box::new(runtime_prereqs::PluginsDirCheck),
        Box::new(gpu_driver_sync::GpuDriverSyncCheck),
        Box::new(plugin_process_leaks::PluginProcessLeaksCheck),
        Box::new(shell_hook_present::ShellHookPresentCheck),
        Box::new(hotkey_shadows::HotkeyShadowsCheck),
        Box::new(plugin_uid_table::PluginUidTableCheck),
        Box::new(orphan_plugin_configs::OrphanPluginConfigsCheck),
        Box::new(config_parse_failures::ConfigParseFailuresCheck),
        Box::new(plugin_port_collisions::PluginPortCollisionsCheck),
    ];
    #[cfg(feature = "dev")]
    let checks = checks.into_iter().chain(dev_checks()).collect();
    checks
}

#[cfg(feature = "dev")]
fn dev_checks() -> impl Iterator<Item = Box<dyn DoctorCheck>> {
    [
        Box::new(cargo_target_cache::CargoTargetCacheCheck) as Box<dyn DoctorCheck>,
        Box::new(cargo_target_total::CargoTargetTotalCheck),
        Box::new(plugin_daemon_health::PluginDaemonHealthCheck),
        Box::new(plugin_staleness::PluginStalenessCheck),
        Box::new(dev_link_paths::DevLinkPathsCheck),
        Box::new(fingerprint_health::FingerprintHealthCheck),
        Box::new(reserved_plugin_ids::ReservedPluginIdsCheck),
        Box::new(single_source_guard::SingleSourceGuardCheck),
        Box::new(rust_formatting::RustFormattingCheck),
        Box::new(rust_clippy::RustClippyCheck),
    ]
    .into_iter()
}
