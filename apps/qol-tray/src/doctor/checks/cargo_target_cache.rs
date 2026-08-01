use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use super::cargo_target::workspace_root;
use qol_dev_build::target_cache::{dir_size, format_bytes};
use std::path::{Path, PathBuf};

const ID: &str = "cargo_target_cache";
const WARN_BYTES: u64 = 8 * 1024 * 1024 * 1024;

pub(super) struct CargoTargetCacheCheck;

impl DoctorCheck for CargoTargetCacheCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Cargo target cache", CheckCategory::DevBuild)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let Some(root) = workspace_root() else {
            return CheckReport::ok("workspace root not found; skipping cargo target cache");
        };
        let path = cargo_incremental_dir(&root);
        report_for(cache_size(&path), path)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CacheSize {
    Missing,
    Bytes(u64),
    Unreadable(String),
}

fn cargo_incremental_dir(root: &Path) -> PathBuf {
    root.join("target").join("debug").join("incremental")
}

fn cache_size(path: &Path) -> CacheSize {
    match dir_size(path) {
        Ok(Some(bytes)) => CacheSize::Bytes(bytes),
        Ok(None) => CacheSize::Missing,
        Err(error) => CacheSize::Unreadable(error),
    }
}

fn report_for(size: CacheSize, path: PathBuf) -> CheckReport {
    match size {
        CacheSize::Missing => CheckReport::ok("cargo incremental cache has not been created yet"),
        CacheSize::Bytes(bytes) if bytes <= WARN_BYTES => CheckReport::ok(format!(
            "cargo incremental cache is {}",
            format_bytes(bytes)
        )),
        CacheSize::Bytes(bytes) => CheckReport::warn(
            format!(
                "cargo incremental cache is {} over the {} limit",
                format_bytes(bytes),
                format_bytes(WARN_BYTES)
            ),
            ID,
            vec![FixAction::PruneCargoIncrementalCache { path }],
        ),
        CacheSize::Unreadable(reason) => CheckReport::ok(format!(
            "cargo incremental cache unreadable, skipping: {reason}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_cache_is_ok_without_fix() {
        let report = report_for(
            CacheSize::Missing,
            PathBuf::from("/target/debug/incremental"),
        );
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn cache_below_limit_is_ok_without_fix() {
        let report = report_for(
            CacheSize::Bytes(WARN_BYTES),
            PathBuf::from("/target/debug/incremental"),
        );
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
        assert!(report.summary.contains("8.0 GiB"));
    }

    #[test]
    fn cache_above_limit_warns_with_incremental_fix() {
        let path = PathBuf::from("/target/debug/incremental");
        let report = report_for(CacheSize::Bytes(WARN_BYTES + 1), path.clone());
        assert_eq!(report.issues.len(), 1);
        assert_eq!(
            report.fixes,
            vec![FixAction::PruneCargoIncrementalCache { path }]
        );
    }
}
