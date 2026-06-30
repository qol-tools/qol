use std::path::{Path, PathBuf};

pub(super) fn workspace_root() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)?;
    root.join("Cargo.toml")
        .is_file()
        .then(|| root.to_path_buf())
}

pub(super) fn dir_size(path: &Path) -> Result<Option<u64>, String> {
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

pub(super) fn format_bytes(bytes: u64) -> String {
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
    fn dir_size_sums_nested_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).expect("nested dir");
        std::fs::write(dir.path().join("root.bin"), vec![0; 3]).expect("root file");
        std::fs::write(nested.join("leaf.bin"), vec![0; 5]).expect("leaf file");

        assert_eq!(dir_size(dir.path()).expect("dir size"), Some(8));
    }

    #[test]
    fn dir_size_reports_missing_path_as_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist");
        assert_eq!(dir_size(&missing).expect("dir size"), None);
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
