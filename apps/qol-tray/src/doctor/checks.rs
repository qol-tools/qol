mod autostart_target;
mod install_identity;
mod runtime_prereqs;

use super::diagnosis::Diagnosis;

pub(super) fn collect_diagnoses() -> Vec<Diagnosis> {
    vec![
        install_identity::check(),
        autostart_target::check(),
        runtime_prereqs::check_plugins_dir(),
    ]
}
