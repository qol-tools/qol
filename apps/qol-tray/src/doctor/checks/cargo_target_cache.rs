use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
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

fn workspace_root() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)?;
    root.join("Cargo.toml")
        .is_file()
        .then(|| root.to_path_buf())
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

fn dir_size(path: &Path) -> Result<Option<u64>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !meta.is_dir() {
        return Ok(Some(meta.len()));
    }
    let entries = std::fs::read_dir(path).map_err(|error| error.to_string())?;
    let mut total = 0;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        if let Some(bytes) = dir_size(&entry.path())? {
            total += bytes;
        }
    }
    Ok(Some(total))
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

fn format_bytes(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes >= GIB {
        return format!("{:.1} GiB", bytes as f64 / GIB as f64);
    }
    if bytes >= MIB {
        return format!("{} MiB", bytes / MIB);
    }
    format!("{} KiB", bytes / 1024)
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

    #[test]
    fn dir_size_sums_nested_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).expect("nested dir");
        std::fs::write(dir.path().join("root.bin"), vec![0; 3]).expect("root file");
        std::fs::write(nested.join("leaf.bin"), vec![0; 5]).expect("leaf file");

        assert_eq!(dir_size(dir.path()).expect("dir size"), Some(8));
    }

    #[test]
    fn workspace_root_resolves_to_dir_with_cargo_toml() {
        let root = workspace_root().expect("workspace root resolves in-tree");
        assert!(
            root.join("Cargo.toml").is_file(),
            "root: {}",
            root.display()
        );
    }
}
