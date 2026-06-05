mod autostart_target;
#[cfg(feature = "dev")]
mod dev_link_paths;
mod hotkey_shadows;
mod install_identity;
mod plugin_process_leaks;
#[cfg(feature = "dev")]
mod plugin_staleness;
mod runtime_prereqs;
mod shell_hook_present;

#[cfg(feature = "dev")]
pub(super) use dev_link_paths::relocate_dev_link;

use super::diagnosis::Diagnosis;
use super::CheckId;

pub(super) fn collect_diagnoses() -> Vec<Diagnosis> {
    #[allow(unused_mut)]
    let mut diagnoses = vec![
        install_identity::check(),
        autostart_target::check(),
        runtime_prereqs::check_plugins_dir(),
        plugin_process_leaks::check(),
        shell_hook_present::check(),
        hotkey_shadows::check(),
    ];
    #[cfg(feature = "dev")]
    {
        diagnoses.push(plugin_staleness::check());
        diagnoses.push(dev_link_paths::check());
    }
    diagnoses
}

pub(super) fn collect_diagnosis(id: CheckId) -> Diagnosis {
    match id {
        CheckId::PluginProcessLeaks => plugin_process_leaks::check(),
        CheckId::HotkeyShadows => hotkey_shadows::check(),
    }
}
