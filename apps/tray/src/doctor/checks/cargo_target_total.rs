use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use super::cargo_target::workspace_root;
use super::doctor_sizes::{self, StoredSize};
use super::ttl_cell::TtlCell;
use qol_dev_build::target_cache::{
    cargo_cache_dirs, dir_size, format_bytes, prunable_target_bytes, INCREMENTAL_CACHE_CEILING,
    SWEPT_CACHE_CEILING,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

const ID: &str = "cargo_target_total";
const WARN_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);

pub(super) struct CargoTargetTotalCheck {
    sizes: TtlCell<(TargetSize, u64, Vec<PathBuf>)>,
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
        let (size, prunable, targets) = self.sizes.get_or_compute(CACHE_TTL, || {
            let targets = cargo_cache_dirs(&root);
            let (size, prunable) = compute_total(&root, &targets, dir_size, prunable_target_bytes);
            (size, prunable, targets)
        });
        report_for(size, prunable, targets)
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
    targets: &[PathBuf],
    mut walk_total: impl FnMut(&Path) -> Result<Option<u64>, String>,
    mut walk_prunable: impl FnMut(&Path) -> u64,
) -> (TargetSize, u64) {
    let now = doctor_sizes::now_ms();
    let path = doctor_sizes::path_for(root);
    if let Some(stored) = doctor_sizes::load(&path) {
        if stored.fresh(now, CACHE_TTL) {
            if let Some(total) = stored.total {
                return (total.into(), stored.prunable);
            }
        }
    }
    let mut size = TargetSize::Missing;
    for target in targets {
        size = match (size, walk_total(target)) {
            (TargetSize::Unreadable(reason), _) | (_, Err(reason)) => {
                TargetSize::Unreadable(reason)
            }
            (size, Ok(None)) => size,
            (TargetSize::Bytes(total), Ok(Some(bytes))) => TargetSize::Bytes(total + bytes),
            (TargetSize::Missing, Ok(Some(bytes))) => TargetSize::Bytes(bytes),
        };
    }
    let prunable = targets.iter().map(|target| walk_prunable(target)).sum();
    let mut sizes = doctor_sizes::load(&path).unwrap_or_default();
    sizes.scanned_at_ms = now;
    sizes.total = Some((&size).into());
    sizes.prunable = prunable;
    doctor_sizes::save(&path, &sizes);
    (size, prunable)
}

fn report_for(size: TargetSize, prunable: u64, targets: Vec<PathBuf>) -> CheckReport {
    match size {
        TargetSize::Missing => CheckReport::ok("cargo target directory has not been created yet"),
        TargetSize::Bytes(bytes) if prunable <= WARN_BYTES => CheckReport::ok(format!(
            "cargo target directory is {} ({} prunable)",
            format_bytes(bytes),
            format_bytes(prunable)
        )),
        TargetSize::Bytes(bytes) => CheckReport::warn(
            format!(
                "cargo target directory is {} with {} prunable; removing stale secondary target roots, debug artifacts over the {} ceiling and incremental caches over the {} ceiling, oldest first",
                format_bytes(bytes),
                format_bytes(prunable),
                format_bytes(SWEPT_CACHE_CEILING),
                format_bytes(INCREMENTAL_CACHE_CEILING)
            ),
            ID,
            vec![FixAction::PruneCargoTargetDir { targets }],
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
        let report = report_for(TargetSize::Missing, 0, vec![PathBuf::from("/repo/target")]);
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn large_target_with_little_stale_weight_is_ok_without_fix() {
        let report = report_for(
            TargetSize::Bytes(20 * WARN_BYTES),
            WARN_BYTES,
            vec![PathBuf::from("/repo/target")],
        );
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
        assert!(report.summary.contains("2.0 GiB prunable"));
    }

    #[test]
    fn stale_weight_above_limit_warns_with_prune_fix() {
        let targets = vec![
            PathBuf::from("/repo/target"),
            PathBuf::from("/git/.cargo-build"),
        ];
        let report = report_for(
            TargetSize::Bytes(20 * WARN_BYTES),
            WARN_BYTES + 1,
            targets.clone(),
        );
        assert_eq!(report.issues.len(), 1);
        assert_eq!(
            report.fixes,
            vec![FixAction::PruneCargoTargetDir { targets }],
            "the prune must never be cargo clean: live dev caches stay protected"
        );
        assert!(report
            .summary
            .contains("incremental caches over the 48.0 GiB ceiling, oldest first"));
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

        let (size, prunable) =
            compute_total(root, &[root.join("target")], total_walk, prunable_walk);

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
        let total_walk = |_: &Path| {
            total_walks += 1;
            Ok(Some(7))
        };
        let prunable_walk = |_: &Path| {
            prunable_walks += 1;
            8
        };

        let (size, prunable) = compute_total(
            root,
            &[root.join("target"), root.join(".cargo-build")],
            total_walk,
            prunable_walk,
        );

        assert_eq!(
            size,
            TargetSize::Bytes(14),
            "the shared build dir counts with target"
        );
        assert_eq!(prunable, 16);
        assert_eq!(total_walks, 2, "a stale cached file must walk again");
        assert_eq!(prunable_walks, 2);
        let stored = doctor_sizes::load(&doctor_sizes::path_for(root)).expect("stored");
        assert_eq!(stored.total, Some(StoredSize::Bytes(14)));
        assert_eq!(stored.prunable, 16);
        assert!(stored.fresh(doctor_sizes::now_ms(), CACHE_TTL));
    }
}
