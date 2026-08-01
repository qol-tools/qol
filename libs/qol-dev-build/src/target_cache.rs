use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

const PROTECTED_TARGET_ROOTS: [&str; 3] = ["debug", "qol-dev", "qol-env"];
const REMOVED_DEBUG_DIRS: [&str; 2] = ["incremental", "examples"];
const SWEPT_DEBUG_DIRS: [&str; 3] = ["deps", "build", ".fingerprint"];
const SWEEP_MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);

pub fn dir_size(path: &Path) -> Result<Option<u64>, String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !meta.is_dir() {
        return Ok(Some(meta.len()));
    }
    let entries = fs::read_dir(path).map_err(|error| error.to_string())?;
    let mut total = 0;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        if let Some(bytes) = dir_size(&entry.path())? {
            total += bytes;
        }
    }
    Ok(Some(total))
}

pub fn path_bytes(path: &Path) -> u64 {
    let Ok(metadata) = path.symlink_metadata() else {
        return 0;
    };
    if !metadata.is_dir() {
        return metadata.len();
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| path_bytes(&entry.path()))
        .sum()
}

pub fn format_bytes(bytes: u64) -> String {
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

pub fn is_protected_target_root(name: &str) -> bool {
    name.starts_with('.') || name == "CACHEDIR.TAG" || PROTECTED_TARGET_ROOTS.contains(&name)
}

pub fn prune_cargo_target_dir(target: &Path) -> Result<(), String> {
    let entries = fs::read_dir(target).map_err(|error| {
        format!(
            "failed to read target directory {}: {error}",
            target.display()
        )
    })?;
    let mut failures = Vec::new();
    for entry in entries.flatten() {
        if is_protected_target_root(&entry.file_name().to_string_lossy()) {
            continue;
        }
        try_remove_target_path(&entry.path(), &mut failures);
    }
    let debug = target.join("debug");
    for name in REMOVED_DEBUG_DIRS {
        try_remove_target_path(&debug.join(name), &mut failures);
    }
    let cutoff = SystemTime::now() - SWEEP_MAX_AGE;
    for name in SWEPT_DEBUG_DIRS {
        sweep_stale_files(&debug.join(name), cutoff, &mut failures);
    }
    if failures.is_empty() {
        return Ok(());
    }
    Err(format!("could not remove: {}", failures.join(", ")))
}

pub fn prunable_target_bytes(target: &Path) -> u64 {
    let mut bytes = 0;
    if let Ok(entries) = fs::read_dir(target) {
        for entry in entries.flatten() {
            if is_protected_target_root(&entry.file_name().to_string_lossy()) {
                continue;
            }
            bytes += path_bytes(&entry.path());
        }
    }
    let debug = target.join("debug");
    for name in REMOVED_DEBUG_DIRS {
        bytes += path_bytes(&debug.join(name));
    }
    let cutoff = SystemTime::now() - SWEEP_MAX_AGE;
    for name in SWEPT_DEBUG_DIRS {
        bytes += stale_file_bytes(&debug.join(name), cutoff);
    }
    bytes
}

fn stale_file_bytes(dir: &Path, cutoff: SystemTime) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut bytes = 0;
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            bytes += stale_file_bytes(&entry.path(), cutoff);
        } else if metadata.modified().is_ok_and(|modified| modified < cutoff) {
            bytes += metadata.len();
        }
    }
    bytes
}

fn try_remove_target_path(path: &Path, failures: &mut Vec<String>) {
    let result = match path.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => failures.push(format!("{} ({error})", path.display())),
    }
}

fn sweep_stale_files(dir: &Path, cutoff: SystemTime, failures: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            failures.push(format!("{} ({error})", dir.display()));
            return;
        }
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            sweep_stale_files(&entry.path(), cutoff, failures);
            continue;
        }
        if metadata.modified().is_ok_and(|modified| modified < cutoff) {
            try_remove_target_path(&entry.path(), failures);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_size_sums_nested_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a").join("b");
        fs::create_dir_all(&nested).expect("nested dir");
        fs::write(dir.path().join("root.bin"), vec![0; 3]).expect("root file");
        fs::write(nested.join("leaf.bin"), vec![0; 5]).expect("leaf file");

        assert_eq!(dir_size(dir.path()).expect("dir size"), Some(8));
        assert_eq!(path_bytes(dir.path()), 8);
    }

    #[test]
    fn dir_size_reports_missing_path_as_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist");
        assert_eq!(dir_size(&missing).expect("dir size"), None);
        assert_eq!(path_bytes(&missing), 0);
    }

    #[test]
    fn target_prune_removes_stale_caches_and_protects_live_dev_paths() {
        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path();
        let stale_mtime = SystemTime::now() - (SWEEP_MAX_AGE + Duration::from_secs(60));
        let cases = [
            ("qol-env/lane/qol-tray", true, true),
            ("release/libfoo.rlib", false, false),
            ("cargo-timings/timing.html", false, false),
            ("debug/incremental/foo/dep-graph.bin", false, false),
            ("debug/examples/demo", false, false),
            ("debug/deps/libold.rlib", true, false),
            ("debug/build/foo/output", true, false),
            ("debug/.fingerprint/old/lib.json", true, false),
            ("debug/deps/libfresh.rlib", false, true),
            ("debug/qol-tray", true, true),
            ("qol-dev/runtime/gen/qol-tray", true, true),
            ("CACHEDIR.TAG", true, true),
            (".rustc_info.json", true, true),
        ];
        for (rel, stale, _) in cases {
            let path = target.join(rel);
            fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            fs::write(&path, b"x").expect("file");
            if stale {
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .expect("open")
                    .set_modified(stale_mtime)
                    .expect("mtime");
            }
        }

        let doomed_bytes = cases.iter().filter(|(_, _, kept)| !kept).count() as u64;
        assert_eq!(prunable_target_bytes(target), doomed_bytes);

        prune_cargo_target_dir(target).expect("prune target");

        for (rel, _, kept) in cases {
            assert_eq!(target.join(rel).exists(), kept, "path: {rel}");
        }
        assert_eq!(prunable_target_bytes(target), 0);
    }

    #[cfg(unix)]
    #[test]
    fn target_prune_continues_past_unremovable_entries() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path();
        let locked = target.join("release");
        fs::create_dir_all(&locked).expect("locked dir");
        fs::write(locked.join("held.rlib"), b"x").expect("held file");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("lock perms");
        let removable = target.join("sandbox");
        fs::create_dir_all(&removable).expect("removable dir");
        fs::write(removable.join("f"), b"x").expect("removable file");

        let error =
            prune_cargo_target_dir(target).expect_err("locked entry must surface as an error");

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("unlock perms");
        assert!(error.contains("release"), "error: {error}");
        assert!(
            !removable.exists(),
            "one unremovable entry must not stop the rest of the prune"
        );
    }
}
