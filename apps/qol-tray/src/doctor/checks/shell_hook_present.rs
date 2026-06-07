use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use crate::installer::{shell_hook_any_rc_exists, shell_hook_status, ShellHookStatus};
use std::path::PathBuf;

const ID: &str = "shell_hook_present";

pub(super) struct ShellHookPresentCheck;

impl DoctorCheck for ShellHookPresentCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Shell hook present", CheckCategory::Install)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let any_rc = match shell_hook_any_rc_exists() {
            Ok(b) => b,
            Err(error) => {
                return CheckReport::error(format!("failed to inspect shell rc files: {error}"), ID)
            }
        };
        let status = match shell_hook_status() {
            Ok(s) => s,
            Err(error) => {
                return CheckReport::error(format!("failed to inspect shell rc files: {error}"), ID)
            }
        };
        diagnose(status, any_rc)
    }
}

fn diagnose(status: ShellHookStatus, any_rc_file_exists: bool) -> CheckReport {
    if !any_rc_file_exists {
        return CheckReport::ok("no shell rc files found; shell hook does not apply".to_string());
    }
    match status {
        ShellHookStatus::AllPresent => {
            CheckReport::ok("qol-tools shell hook present in all rc files".to_string())
        }
        ShellHookStatus::PartialMissing(paths) => CheckReport::warn(
            format!("qol-tools shell hook missing from {}", format_paths(&paths)),
            ID,
            vec![FixAction::InstallShellHook],
        ),
        ShellHookStatus::NoneInstalled => CheckReport::warn(
            "qol-tools shell hook missing from rc files".to_string(),
            ID,
            vec![FixAction::InstallShellHook],
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

    #[test]
    fn diagnose_table() {
        let cases: Vec<(ShellHookStatus, bool, usize, usize)> = vec![
            (ShellHookStatus::AllPresent, true, 0, 0),
            (ShellHookStatus::NoneInstalled, false, 0, 0),
            (ShellHookStatus::NoneInstalled, true, 1, 1),
            (
                ShellHookStatus::PartialMissing(vec![PathBuf::from("/home/u/.bashrc")]),
                true,
                1,
                1,
            ),
        ];
        for (status, any_rc, expected_issues, expected_fixes) in cases {
            let report = diagnose(status, any_rc);
            assert_eq!(
                report.issues.len(),
                expected_issues,
                "issues mismatch any_rc={any_rc}"
            );
            assert_eq!(
                report.fixes.len(),
                expected_fixes,
                "fixes mismatch any_rc={any_rc}"
            );
            if expected_fixes > 0 {
                assert!(matches!(
                    report.fixes.as_slice(),
                    [FixAction::InstallShellHook]
                ));
            }
        }
    }
}
