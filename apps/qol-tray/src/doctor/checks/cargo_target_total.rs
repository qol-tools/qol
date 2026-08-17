use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use super::cargo_target::workspace_root;
use super::ttl_cell::TtlCell;
use qol_dev_build::target_cache::{
    dir_size, format_bytes, prunable_target_bytes, SWEPT_CACHE_CEILING,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

const ID: &str = "cargo_target_total";
const WARN_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);

pub(super) struct CargoTargetTotalCheck {
    sizes: TtlCell<(TargetSize, u64)>,
}

impl CargoTargetTotalCheck {
    pub(super) fn new() -> Self {
        Self {
            sizes: TtlCell::new(),
        }
    }
}

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
        let (size, prunable) = self.sizes.get_or_compute(CACHE_TTL, || {
            (target_size(&path), prunable_target_bytes(&path))
        });
        report_for(size, prunable, path)
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

fn report_for(size: TargetSize, prunable: u64, path: PathBuf) -> CheckReport {
    match size {
        TargetSize::Missing => CheckReport::ok("cargo target directory has not been created yet"),
        TargetSize::Bytes(bytes) if prunable <= WARN_BYTES => CheckReport::ok(format!(
            "cargo target directory is {} ({} prunable)",
            format_bytes(bytes),
            format_bytes(prunable)
        )),
        TargetSize::Bytes(bytes) => CheckReport::warn(
            format!(
                "cargo target directory is {} with {} prunable; removing secondary target roots, incremental caches, and the oldest debug artifacts over the {} ceiling",
                format_bytes(bytes),
                format_bytes(prunable),
                format_bytes(SWEPT_CACHE_CEILING)
            ),
            ID,
            vec![FixAction::PruneCargoTargetDir { target: path }],
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
        let report = report_for(TargetSize::Missing, 0, PathBuf::from("/repo/target"));
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn large_target_with_little_stale_weight_is_ok_without_fix() {
        let report = report_for(
            TargetSize::Bytes(20 * WARN_BYTES),
            WARN_BYTES,
            PathBuf::from("/repo/target"),
        );
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
        assert!(report.summary.contains("2.0 GiB prunable"));
    }

    #[test]
    fn stale_weight_above_limit_warns_with_prune_fix() {
        let path = PathBuf::from("/repo/target");
        let report = report_for(
            TargetSize::Bytes(20 * WARN_BYTES),
            WARN_BYTES + 1,
            path.clone(),
        );
        assert_eq!(report.issues.len(), 1);
        assert_eq!(
            report.fixes,
            vec![FixAction::PruneCargoTargetDir { target: path }],
            "the prune must never be cargo clean: live dev caches stay protected"
        );
        assert!(report
            .summary
            .contains("oldest debug artifacts over the 48.0 GiB ceiling"));
    }
}
