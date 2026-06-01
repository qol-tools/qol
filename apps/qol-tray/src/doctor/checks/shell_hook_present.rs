use super::super::diagnosis::{error_outcome, ok_outcome, warn_outcome, Diagnosis, FixAction};
use crate::installer::{shell_hook_any_rc_exists, shell_hook_status, ShellHookStatus};
use std::path::PathBuf;

const ID: &str = "shell_hook_present";

pub(super) fn check() -> Diagnosis {
    let any_rc = match shell_hook_any_rc_exists() {
        Ok(b) => b,
        Err(error) => {
            return error_outcome(ID, format!("failed to inspect shell rc files: {error}"))
        }
    };
    let status = match shell_hook_status() {
        Ok(s) => s,
        Err(error) => {
            return error_outcome(ID, format!("failed to inspect shell rc files: {error}"))
        }
    };
    diagnose(status, any_rc)
}

fn diagnose(status: ShellHookStatus, any_rc_file_exists: bool) -> Diagnosis {
    if !any_rc_file_exists {
        return ok_outcome(
            ID,
            "no shell rc files found; shell hook does not apply".to_string(),
        );
    }
    match status {
        ShellHookStatus::AllPresent => ok_outcome(
            ID,
            "qol-tools shell hook present in all rc files".to_string(),
        ),
        ShellHookStatus::PartialMissing(paths) => warn_outcome(
            ID,
            format!("qol-tools shell hook missing from {}", format_paths(&paths)),
            Some(FixAction::InstallShellHook),
        ),
        ShellHookStatus::NoneInstalled => warn_outcome(
            ID,
            "qol-tools shell hook missing from rc files".to_string(),
            Some(FixAction::InstallShellHook),
        ),
    }
}

fn format_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::OutcomeStatus;

    #[test]
    fn diagnose_table() {
        let cases: Vec<(ShellHookStatus, bool, OutcomeStatus, bool)> = vec![
            (ShellHookStatus::AllPresent, true, OutcomeStatus::Ok, false),
            (
                ShellHookStatus::NoneInstalled,
                false,
                OutcomeStatus::Ok,
                false,
            ),
            (
                ShellHookStatus::NoneInstalled,
                true,
                OutcomeStatus::Warn,
                true,
            ),
            (
                ShellHookStatus::PartialMissing(vec![PathBuf::from("/home/u/.bashrc")]),
                true,
                OutcomeStatus::Warn,
                true,
            ),
        ];
        for (status, any_rc, expected_status, expected_fix) in cases {
            let diagnosis = diagnose(status, any_rc);
            assert_eq!(
                diagnosis.outcome.status, expected_status,
                "status mismatch any_rc={any_rc}"
            );
            assert_eq!(
                diagnosis.outcome.fix_available, expected_fix,
                "fix_available mismatch any_rc={any_rc}"
            );
            if expected_fix {
                assert!(matches!(
                    diagnosis.fixes.as_slice(),
                    [FixAction::InstallShellHook]
                ));
            } else {
                assert!(diagnosis.fixes.is_empty());
            }
        }
    }
}
