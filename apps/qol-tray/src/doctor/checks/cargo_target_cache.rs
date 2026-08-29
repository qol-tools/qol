use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use super::cargo_target::workspace_root;
use super::doctor_sizes::{self, StoredSize};
use super::ttl_cell::TtlCell;
use qol_dev_build::target_cache::{dir_size, format_bytes};
use std::path::{Path, PathBuf};
use std::time::Duration;

const ID: &str = "cargo_target_cache";
const WARN_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);

pub(super) struct CargoTargetCacheCheck {
    sizes: TtlCell<CacheSize>,
}

impl CargoTargetCacheCheck {
    pub(super) fn new() -> Self {
        Self {
            sizes: TtlCell::new(),
        }
    }
}

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
        let size = self
            .sizes
            .get_or_compute(CACHE_TTL, || compute_cache(&root, dir_size));
        report_for(size)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CacheSize {
    Missing,
    Bytes(u64),
    Unreadable(String),
}

impl From<StoredSize> for CacheSize {
    fn from(size: StoredSize) -> Self {
        match size {
            StoredSize::Missing => CacheSize::Missing,
            StoredSize::Bytes(bytes) => CacheSize::Bytes(bytes),
            StoredSize::Unreadable(reason) => CacheSize::Unreadable(reason),
        }
    }
}

impl From<&CacheSize> for StoredSize {
    fn from(size: &CacheSize) -> Self {
        match size {
            CacheSize::Missing => StoredSize::Missing,
            CacheSize::Bytes(bytes) => StoredSize::Bytes(*bytes),
            CacheSize::Unreadable(reason) => StoredSize::Unreadable(reason.clone()),
        }
    }
}

fn compute_cache(
    root: &Path,
    walk: impl FnOnce(&Path) -> Result<Option<u64>, String>,
) -> CacheSize {
    let path = cargo_incremental_dir(root);
    let now = doctor_sizes::now_ms();
    let file_path = doctor_sizes::path_for(root);
    if let Some(stored) = doctor_sizes::load(&file_path) {
        if stored.fresh(now, CACHE_TTL) {
            if let Some(cache) = stored.cache {
                return cache.into();
            }
        }
    }
    let size = match walk(&path) {
        Ok(Some(bytes)) => CacheSize::Bytes(bytes),
        Ok(None) => CacheSize::Missing,
        Err(reason) => CacheSize::Unreadable(reason),
    };
    let mut sizes = doctor_sizes::load(&file_path).unwrap_or_default();
    sizes.scanned_at_ms = now;
    sizes.cache = Some((&size).into());
    doctor_sizes::save(&file_path, &sizes);
    size
}

fn cargo_incremental_dir(root: &Path) -> PathBuf {
    root.join("target").join("debug").join("incremental")
}

fn report_for(size: CacheSize) -> CheckReport {
    match size {
        CacheSize::Missing => CheckReport::ok("cargo incremental cache has not been created yet"),
        CacheSize::Bytes(bytes) if bytes <= WARN_BYTES => CheckReport::ok(format!(
            "cargo incremental cache is {}",
            format_bytes(bytes)
        )),
        CacheSize::Bytes(bytes) => CheckReport::warn(
            format!(
                "cargo incremental cache is {} over the {} limit; it is kept because deleting it forces cold rebuilds",
                format_bytes(bytes),
                format_bytes(WARN_BYTES)
            ),
            ID,
            vec![],
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
        let report = report_for(CacheSize::Missing);
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn cache_below_limit_is_ok_without_fix() {
        let report = report_for(CacheSize::Bytes(WARN_BYTES));
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
        assert!(report.summary.contains("8.0 GiB"));
    }

    #[test]
    fn cache_above_limit_warns_without_fix() {
        let report = report_for(CacheSize::Bytes(WARN_BYTES + 1));
        assert_eq!(report.issues.len(), 1);
        assert!(report.fixes.is_empty());
        assert!(report
            .summary
            .contains("it is kept because deleting it forces cold rebuilds"));
    }

    #[test]
    fn fresh_cached_file_skips_the_walk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        doctor_sizes::save(
            &doctor_sizes::path_for(root),
            &doctor_sizes::DoctorSizes {
                scanned_at_ms: doctor_sizes::now_ms(),
                cache: Some(StoredSize::Bytes(2468)),
                ..doctor_sizes::DoctorSizes::default()
            },
        );
        let mut walks = 0;
        let walk = |_: &Path| {
            walks += 1;
            Ok(Some(1))
        };

        let size = compute_cache(root, walk);

        assert_eq!(size, CacheSize::Bytes(2468));
        assert_eq!(walks, 0, "a fresh cached file must skip the walk");
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
                cache: Some(StoredSize::Bytes(2468)),
                ..doctor_sizes::DoctorSizes::default()
            },
        );
        let mut walks = 0;
        let walk = |path: &Path| {
            walks += 1;
            assert!(path.ends_with("incremental"));
            Ok(Some(5))
        };

        let size = compute_cache(root, walk);

        assert_eq!(size, CacheSize::Bytes(5));
        assert_eq!(walks, 1, "a stale cached file must walk again");
        let stored = doctor_sizes::load(&doctor_sizes::path_for(root)).expect("stored");
        assert_eq!(stored.cache, Some(StoredSize::Bytes(5)));
        assert!(stored.fresh(doctor_sizes::now_ms(), CACHE_TTL));
    }
}
