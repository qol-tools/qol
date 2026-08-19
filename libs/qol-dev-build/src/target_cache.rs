use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

const PROTECTED_TARGET_ROOTS: [&str; 3] = ["debug", "qol-dev", "qol-env"];
const REMOVED_DEBUG_DIRS: [&str; 2] = ["incremental", "examples"];
const SWEPT_DEBUG_DIRS: [&str; 3] = ["deps", "build", ".fingerprint"];
pub const SWEPT_CACHE_CEILING: u64 = 48 * 1024 * 1024 * 1024;

const PACE_BATCH: usize = 2000;

pub(crate) struct Pacer {
    visited: u64,
    sleep: fn(Duration),
}

impl Pacer {
    pub(crate) fn new() -> Self {
        Self {
            visited: 0,
            sleep: std::thread::sleep,
        }
    }

    fn pace_sleep() -> Duration {
        use crate::platform::BuildPlatform;
        crate::platform::Platform.walk_pace_sleep()
    }

    #[cfg(test)]
    fn new_with_sleep(sleep: fn(Duration)) -> Self {
        Self { visited: 0, sleep }
    }

    pub(crate) fn tick(&mut self) {
        self.visited += 1;
        if !self.visited.is_multiple_of(PACE_BATCH as u64) {
            return;
        }
        let pace = Self::pace_sleep();
        if !pace.is_zero() {
            (self.sleep)(pace);
        }
    }
}

fn dir_size_paced(path: &Path, pacer: &mut Pacer) -> Result<Option<u64>, String> {
    pacer.tick();
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
        if let Some(bytes) = dir_size_paced(&entry.path(), pacer)? {
            total += bytes;
        }
    }
    Ok(Some(total))
}

pub fn dir_size(path: &Path) -> Result<Option<u64>, String> {
    dir_size_paced(path, &mut Pacer::new())
}

const WALK_THREADS: usize = 4;

pub fn path_bytes(path: &Path) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Condvar, Mutex};
    let Ok(metadata) = path.symlink_metadata() else {
        return 0;
    };
    if !metadata.is_dir() {
        return metadata.len();
    }
    let queue: Mutex<(Vec<std::path::PathBuf>, usize)> = Mutex::new((vec![path.to_path_buf()], 1));
    let idle = Condvar::new();
    let total = AtomicU64::new(0);
    std::thread::scope(|scope| {
        for _ in 0..WALK_THREADS {
            scope.spawn(|| {
                let mut pacer = Pacer::new();
                loop {
                    let dir = {
                        let Ok(mut state) = queue.lock() else {
                            return;
                        };
                        loop {
                            if let Some(dir) = state.0.pop() {
                                break dir;
                            }
                            if state.1 == 0 {
                                return;
                            }
                            state = match idle.wait(state) {
                                Ok(state) => state,
                                Err(_) => return,
                            };
                        }
                    };
                    let mut local = 0u64;
                    let mut subdirs = Vec::new();
                    if let Ok(entries) = fs::read_dir(&dir) {
                        for entry in entries.flatten() {
                            pacer.tick();
                            let child = entry.path();
                            let Ok(meta) = child.symlink_metadata() else {
                                continue;
                            };
                            if meta.is_dir() {
                                subdirs.push(child);
                            } else {
                                local += meta.len();
                            }
                        }
                    }
                    total.fetch_add(local, Ordering::Relaxed);
                    if let Ok(mut state) = queue.lock() {
                        state.1 += subdirs.len();
                        state.0.extend(subdirs);
                        state.1 -= 1;
                        idle.notify_all();
                    }
                }
            });
        }
    });
    total.load(Ordering::Relaxed)
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
    prune_with_ceiling(target, SWEPT_CACHE_CEILING)
}

pub fn prunable_target_bytes(target: &Path) -> u64 {
    prunable_with_ceiling(target, SWEPT_CACHE_CEILING)
}

fn prune_with_ceiling(target: &Path, ceiling: u64) -> Result<(), String> {
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
    evict_oldest_swept_files(&debug, ceiling, &mut failures);
    if failures.is_empty() {
        return Ok(());
    }
    Err(format!("could not remove: {}", failures.join(", ")))
}

fn prunable_with_ceiling(target: &Path, ceiling: u64) -> u64 {
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
    let swept_total: u64 = swept_files(&debug).iter().map(|file| file.bytes).sum();
    bytes + swept_total.saturating_sub(ceiling)
}

struct SweptFile {
    path: std::path::PathBuf,
    modified: SystemTime,
    bytes: u64,
}

fn swept_files(debug: &Path) -> Vec<SweptFile> {
    let mut files = Vec::new();
    for name in SWEPT_DEBUG_DIRS {
        collect_files(&debug.join(name), &mut files);
    }
    files
}

fn collect_files(dir: &Path, files: &mut Vec<SweptFile>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            collect_files(&entry.path(), files);
            continue;
        }
        files.push(SweptFile {
            path: entry.path(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            bytes: metadata.len(),
        });
    }
}

fn evict_oldest_swept_files(debug: &Path, ceiling: u64, failures: &mut Vec<String>) {
    let mut files = swept_files(debug);
    let mut remaining: u64 = files.iter().map(|file| file.bytes).sum();
    if remaining <= ceiling {
        return;
    }
    files.sort_by_key(|file| file.modified);
    for file in files {
        if remaining <= ceiling {
            return;
        }
        let before = failures.len();
        try_remove_target_path(&file.path, failures);
        if failures.len() == before {
            remaining -= file.bytes;
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    #[test]
    fn pacer_sleeps_once_per_batch_where_the_platform_paces() {
        static SLEEPS: AtomicU32 = AtomicU32::new(0);
        fn fake_sleep(_: Duration) {
            SLEEPS.fetch_add(1, Ordering::SeqCst);
        }
        let per_batch = u32::from(!Pacer::pace_sleep().is_zero());
        let mut pacer = Pacer::new_with_sleep(fake_sleep);
        for _ in 0..PACE_BATCH {
            pacer.tick();
        }
        assert_eq!(
            SLEEPS.load(Ordering::SeqCst),
            per_batch,
            "one sleep per batch, none on a kernel-throttled platform"
        );
        for _ in 0..(PACE_BATCH / 2) {
            pacer.tick();
        }
        assert_eq!(
            SLEEPS.load(Ordering::SeqCst),
            per_batch,
            "a partial batch must not sleep"
        );
        for _ in 0..(PACE_BATCH / 2) {
            pacer.tick();
        }
        assert_eq!(
            SLEEPS.load(Ordering::SeqCst),
            per_batch * 2,
            "floor(N / PACE_BATCH) sleeps for N ticks"
        );
    }

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
    fn target_prune_evicts_oldest_caches_to_the_ceiling_and_protects_live_dev_paths() {
        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path();
        let mtime_of = |age_rank: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(age_rank);
        let cases = [
            ("qol-env/lane/qol-tray", None, true),
            ("release/libfoo.rlib", None, false),
            ("cargo-timings/timing.html", None, false),
            ("debug/incremental/foo/dep-graph.bin", None, false),
            ("debug/examples/demo", None, false),
            ("debug/deps/libold.rlib", Some(1), false),
            ("debug/build/foo/output", Some(2), true),
            ("debug/.fingerprint/new/lib.json", Some(3), true),
            ("debug/qol-tray", None, true),
            ("qol-dev/runtime/gen/qol-tray", None, true),
            ("CACHEDIR.TAG", None, true),
            (".rustc_info.json", None, true),
        ];
        for (rel, age_rank, _) in cases {
            let path = target.join(rel);
            fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            fs::write(&path, b"xxxx").expect("file");
            if let Some(age_rank) = age_rank {
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .expect("open")
                    .set_modified(mtime_of(age_rank))
                    .expect("mtime");
            }
        }
        let ceiling = 8;

        let removed_roots = 4 * 4;
        let swept_excess = 3 * 4 - ceiling;
        assert_eq!(
            prunable_with_ceiling(target, ceiling),
            removed_roots + swept_excess,
            "prunable must count removed roots plus the LRU excess over the ceiling"
        );

        prune_with_ceiling(target, ceiling).expect("prune target");

        for (rel, _, kept) in cases {
            assert_eq!(target.join(rel).exists(), kept, "path: {rel}");
        }
        assert_eq!(prunable_with_ceiling(target, ceiling), 0);
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
