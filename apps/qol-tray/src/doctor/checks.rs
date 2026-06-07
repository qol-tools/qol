mod autostart_target;
#[cfg(feature = "dev")]
mod dev_link_paths;
#[cfg(feature = "dev")]
mod fingerprint_health;
mod hotkey_shadows;
mod install_identity;
mod plugin_process_leaks;
#[cfg(feature = "dev")]
mod plugin_staleness;
mod runtime_prereqs;
mod shell_hook_present;

#[cfg(feature = "dev")]
pub(super) use dev_link_paths::relocate_dev_link;

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
    ];
    #[cfg(feature = "dev")]
    {
        checks.push(Box::new(plugin_staleness::PluginStalenessCheck));
        checks.push(Box::new(dev_link_paths::DevLinkPathsCheck));
        checks.push(Box::new(fingerprint_health::FingerprintHealthCheck));
    }
    checks
}
