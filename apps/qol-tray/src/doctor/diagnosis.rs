use super::install_id::write_install_id_file;
use super::report::{Outcome, OutcomeStatus};
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

pub(super) struct Diagnosis {
    pub(super) outcome: Outcome,
    pub(super) fix: Option<FixAction>,
}

pub(super) enum FixAction {
    SetActiveInstallId(String),
    WriteInstallMarker {
        marker_path: PathBuf,
        install_id: String,
    },
    WriteAutostartEntry {
        binary_path: PathBuf,
    },
    EnsurePluginsDir {
        path: PathBuf,
    },
}

pub(super) fn apply_fix(action: &FixAction) -> Result<()> {
    match action {
        FixAction::SetActiveInstallId(install_id) => {
            crate::paths::set_active_install_id(install_id)
        }
        FixAction::WriteInstallMarker {
            marker_path,
            install_id,
        } => write_install_id_file(marker_path, install_id),
        FixAction::WriteAutostartEntry { binary_path } => {
            crate::installer::write_autostart_entry(binary_path)
        }
        FixAction::EnsurePluginsDir { path } => {
            fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))
        }
    }
}

pub(super) fn ok_outcome(id: &'static str, message: String) -> Diagnosis {
    Diagnosis {
        outcome: Outcome {
            id,
            status: OutcomeStatus::Ok,
            message,
            fix_available: false,
        },
        fix: None,
    }
}

pub(super) fn warn_outcome(id: &'static str, message: String, fix: Option<FixAction>) -> Diagnosis {
    Diagnosis {
        outcome: Outcome {
            id,
            status: OutcomeStatus::Warn,
            message,
            fix_available: fix.is_some(),
        },
        fix,
    }
}

pub(super) fn error_outcome(id: &'static str, message: String) -> Diagnosis {
    Diagnosis {
        outcome: Outcome {
            id,
            status: OutcomeStatus::Error,
            message,
            fix_available: false,
        },
        fix: None,
    }
}
