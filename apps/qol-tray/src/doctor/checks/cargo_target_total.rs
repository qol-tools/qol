use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use super::cargo_target::workspace_root;
use super::doctor_sizes::{self, StoredSize};
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
        let (size, prunable) = self.sizes.get_or_compute(CACHE_TTL, || {
            compute_total(&root, dir_size, prunable_target_bytes)
        });
        report_for(size, prunable, root.join("target"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TargetSize {
    Missing,
    Bytes(u64),
    Unreadable(String),
}

impl From<StoredSize> for TargetSize {
    fn from(size: StoredSize) -> Self {
        match size {
            StoredSize::Missing => TargetSize::Missing,
            StoredSize::Bytes(bytes) => TargetSize::Bytes(bytes),
            StoredSize::Unreadable(reason) => TargetSize::Unreadable(reason),
        }
    }
}

impl From<&TargetSize> for StoredSize {
    fn from(size: &TargetSize) -> Self {
        match size {
            TargetSize::Missing => StoredSize::Missing,
            TargetSize::Bytes(bytes) => StoredSize::Bytes(*bytes),
            TargetSize::Unreadable(reason) => StoredSize::Unreadable(reason.clone()),
        }
    }
}

fn compute_total(
    root: &Path,
    walk_total: impl FnOnce(&Path) -> Result<Option<u64>, String>,
    walk_prunable: impl FnOnce(&Path) -> u64,
) -> (TargetSize, u64) {
    let target = root.join("target");
    let now = doctor_sizes::now_ms();
    let path = doctor_sizes::path_for(root);
    if let Some(stored) = doctor_sizes::load(&path) {
        if stored.fresh(now, CACHE_TTL) {
            if let Some(total) = stored.total {
                return (total.into(), stored.prunable);
            }
        }
    }
    let size = match walk_total(&target) {
        Ok(Some(bytes)) => TargetSize::Bytes(bytes),
        Ok(None) => TargetSize::Missing,
        Err(reason) => TargetSize::Unreadable(reason),
    };
    let prunable = walk_prunable(&target);
    let mut sizes = doctor_sizes::load(&path).unwrap_or_default();
    sizes.scanned_at_ms = now;
    sizes.total = Some((&size).into());
    sizes.prunable = prunable;
    doctor_sizes::save(&path, &sizes);
    (size, prunable)
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
                "cargo target directory is {} with {} prunable; removing stale secondary target roots and the oldest debug artifacts over the {} ceiling",
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

    #[test]
    fn fresh_cached_file_skips_both_walks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        doctor_sizes::save(
            &doctor_sizes::path_for(root),
            &doctor_sizes::DoctorSizes {
                scanned_at_ms: doctor_sizes::now_ms(),
                total: Some(StoredSize::Bytes(4321)),
                prunable: 99,
                ..doctor_sizes::DoctorSizes::default()
            },
        );
        let mut total_walks = 0;
        let mut prunable_walks = 0;
        let total_walk = |_: &Path| {
            total_walks += 1;
            Ok(Some(1))
        };
        let prunable_walk = |_: &Path| {
            prunable_walks += 1;
            2
        };

        let (size, prunable) = compute_total(root, total_walk, prunable_walk);

        assert_eq!(size, TargetSize::Bytes(4321));
        assert_eq!(prunable, 99);
        assert_eq!(
            total_walks, 0,
            "a fresh cached file must skip the size walk"
        );
        assert_eq!(
            prunable_walks, 0,
            "a fresh cached file must skip the prunable walk"
        );
    }

    #[test]
    fn stale_cached_file_rewalks_and_refreshes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        doctor_sizes::save(
            &doctor_sizes::path_for(root),
            &doctor_sizes::DoctorSizes {
                scanned_at_ms: doctor_sizes::now_ms()
                    .saturating_sub(CACHE_TTL.as_millis() as u64 + 1_000),
                total: Some(StoredSize::Bytes(4321)),
                prunable: 99,
                ..doctor_sizes::DoctorSizes::default()
            },
        );
        let mut total_walks = 0;
        let mut prunable_walks = 0;
        let total_walk = |path: &Path| {
            total_walks += 1;
            assert!(path.ends_with("target"));
            Ok(Some(7))
        };
        let prunable_walk = |_: &Path| {
            prunable_walks += 1;
            8
        };

        let (size, prunable) = compute_total(root, total_walk, prunable_walk);

        assert_eq!(size, TargetSize::Bytes(7));
        assert_eq!(prunable, 8);
        assert_eq!(total_walks, 1, "a stale cached file must walk again");
        assert_eq!(prunable_walks, 1);
        let stored = doctor_sizes::load(&doctor_sizes::path_for(root)).expect("stored");
        assert_eq!(stored.total, Some(StoredSize::Bytes(7)));
        assert_eq!(stored.prunable, 8);
        assert!(stored.fresh(doctor_sizes::now_ms(), CACHE_TTL));
    }
}
