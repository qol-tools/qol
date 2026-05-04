mod autostart_target;
#[cfg(target_os = "linux")]
mod hotkey_shadows;
mod install_identity;
mod plugin_process_leaks;
#[cfg(feature = "dev")]
mod plugin_staleness;
mod runtime_prereqs;

use super::diagnosis::Diagnosis;
use super::CheckId;

pub(super) fn collect_diagnoses() -> Vec<Diagnosis> {
    // `mut` is conditionally needed (linux push, dev push). Suppress
    // unused_mut on builds where neither branch fires.
    #[allow(unused_mut)]
    let mut diagnoses = vec![
        install_identity::check(),
        autostart_target::check(),
        runtime_prereqs::check_plugins_dir(),
        plugin_process_leaks::check(),
    ];
    #[cfg(target_os = "linux")]
    diagnoses.push(hotkey_shadows::check());
    #[cfg(feature = "dev")]
    diagnoses.push(plugin_staleness::check());
    diagnoses
}

pub(super) fn collect_diagnosis(id: CheckId) -> Diagnosis {
    match id {
        CheckId::PluginProcessLeaks => plugin_process_leaks::check(),
    }
}
