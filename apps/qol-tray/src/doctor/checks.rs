mod autostart_target;
#[cfg(feature = "dev")]
mod cargo_target;
#[cfg(feature = "dev")]
mod cargo_target_cache;
#[cfg(feature = "dev")]
mod cargo_target_total;
#[cfg(feature = "dev")]
mod dev_link_paths;
#[cfg(feature = "dev")]
mod fingerprint_health;
mod hotkey_shadows;
mod install_identity;
mod orphan_plugin_configs;
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
pub(super) use dev_link_paths::relocate_dev_link;
#[cfg(feature = "dev")]
pub(super) use plugin_staleness::stale_running_daemons;

use super::framework::DoctorCheck;

pub(super) fn registry() -> Vec<Box<dyn DoctorCheck>> {
    #[allow(unused_mut)]
    let mut checks: Vec<Box<dyn DoctorCheck>> = vec![
        Box::new(install_identity::InstallIdentityCheck),
        Box::new(autostart_target::AutostartTargetCheck),
        Box::new(runtime_prereqs::PluginsDirCheck),
        Box::new(plugin_process_leaks::PluginProcessLeaksCheck),
        Box::new(shell_hook_present::ShellHookPresentCheck),
        Box::new(hotkey_shadows::HotkeyShadowsCheck),
        Box::new(plugin_uid_table::PluginUidTableCheck),
        Box::new(orphan_plugin_configs::OrphanPluginConfigsCheck),
        Box::new(plugin_port_collisions::PluginPortCollisionsCheck),
    ];
    #[cfg(feature = "dev")]
    {
        checks.push(Box::new(cargo_target_cache::CargoTargetCacheCheck));
        checks.push(Box::new(cargo_target_total::CargoTargetTotalCheck));
        checks.push(Box::new(plugin_staleness::PluginStalenessCheck));
        checks.push(Box::new(dev_link_paths::DevLinkPathsCheck));
        checks.push(Box::new(fingerprint_health::FingerprintHealthCheck));
        checks.push(Box::new(reserved_plugin_ids::ReservedPluginIdsCheck));
        checks.push(Box::new(rust_formatting::RustFormattingCheck));
        checks.push(Box::new(rust_clippy::RustClippyCheck));
    }
    checks
}
