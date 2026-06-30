use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use super::cargo_target::{dir_size, format_bytes, workspace_root};
use std::path::{Path, PathBuf};

const ID: &str = "cargo_target_total";
const WARN_BYTES: u64 = 10 * 1024 * 1024 * 1024;

pub(super) struct CargoTargetTotalCheck;

impl DoctorCheck for CargoTargetTotalCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Cargo target directory", CheckCategory::DevBuild)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let Some(root) = workspace_root() else {
            return CheckReport::ok("workspace root not found; skipping cargo target directory");
        };
        let path = root.join("target");
        report_for(target_size(&path), root)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TargetSize {
    Missing,
    Bytes(u64),
    Unreadable(String),
}

fn target_size(path: &Path) -> TargetSize {
    match dir_size(path) {
        Ok(Some(bytes)) => TargetSize::Bytes(bytes),
        Ok(None) => TargetSize::Missing,
        Err(error) => TargetSize::Unreadable(error),
    }
}

fn report_for(size: TargetSize, workspace: PathBuf) -> CheckReport {
    match size {
        TargetSize::Missing => CheckReport::ok("cargo target directory has not been created yet"),
        TargetSize::Bytes(bytes) if bytes <= WARN_BYTES => {
            CheckReport::ok(format!("cargo target directory is {}", format_bytes(bytes)))
        }
        TargetSize::Bytes(bytes) => CheckReport::warn(
            format!(
                "cargo target directory is {} over the {} limit; cargo clean reclaims it",
                format_bytes(bytes),
                format_bytes(WARN_BYTES)
            ),
            ID,
            vec![FixAction::CargoClean { workspace }],
        ),
        TargetSize::Unreadable(reason) => CheckReport::ok(format!(
            "cargo target directory unreadable, skipping: {reason}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_target_is_ok_without_fix() {
        let report = report_for(TargetSize::Missing, PathBuf::from("/ws"));
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn target_below_limit_is_ok_without_fix() {
        let report = report_for(TargetSize::Bytes(WARN_BYTES), PathBuf::from("/ws"));
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
        assert!(report.summary.contains("10.0 GiB"));
    }

    #[test]
    fn target_above_limit_warns_with_cargo_clean_fix() {
        let workspace = PathBuf::from("/ws");
        let report = report_for(TargetSize::Bytes(WARN_BYTES + 1), workspace.clone());
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.fixes, vec![FixAction::CargoClean { workspace }]);
    }
}
