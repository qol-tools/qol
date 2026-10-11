mod apply;
mod files;
mod incremental;
mod plan;
mod units;

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

const PROTECTED_TARGET_ROOTS: [&str; 7] = [
    "debug",
    "qol-dev",
    "qol-env",
    "release",
    "qol-check",
    "qol-lint",
    "sherpa-onnx-prebuilt",
];
const BUILD_LOCK_FILES: [&str; 3] = [".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock"];
const PROFILE_DIRS: [&str; 2] = ["debug", "release"];
const SWEEP_LOCK_FILE: &str = ".qol-sweep.lock";
pub const KEEP_UNUSED_FOR: Duration = Duration::from_secs(3 * 24 * 60 * 60);

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

pub fn cargo_cache_dirs(root: &Path) -> Vec<PathBuf> {
    let target = root.join("target");
    match cargo_metadata(root).and_then(|metadata| separate_build_dir(&metadata)) {
        Some(build_dir) => vec![target, build_dir],
        None => vec![target],
    }
}

fn cargo_metadata(root: &Path) -> Option<serde_json::Value> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--offline",
            "--format-version",
            "1",
        ])
        .current_dir(root)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

fn separate_build_dir(metadata: &serde_json::Value) -> Option<PathBuf> {
    let build_dir = metadata.get("build_directory")?.as_str()?;
    let target_dir = metadata.get("target_directory")?.as_str()?;
    (build_dir != target_dir).then(|| PathBuf::from(build_dir))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SweepMode {
    DryRun,
    Apply,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    pub count: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SweepReport {
    pub dir: PathBuf,
    pub dry_run: bool,
    pub access_times: bool,
    pub keep_unused_days: u64,
    pub freed_bytes: u64,
    pub units: Tally,
    pub superseded_sessions: Tally,
    pub idle_crate_caches: Tally,
    pub stale_roots: Tally,
    pub skipped: Vec<String>,
    pub failures: Vec<String>,
}

pub fn sweep_cargo_cache_dir(dir: &Path, mode: SweepMode) -> Result<SweepReport, String> {
    sweep(dir, mode, SystemTime::now(), KEEP_UNUSED_FOR, None)
}

pub fn prune_cargo_target_dir(target: &Path) -> Result<SweepReport, String> {
    let report = sweep_cargo_cache_dir(target, SweepMode::Apply)?;
    let problems: Vec<&str> = report
        .skipped
        .iter()
        .chain(&report.failures)
        .map(String::as_str)
        .collect();
    if problems.is_empty() {
        return Ok(report);
    }
    Err(problems.join("; "))
}

pub fn prunable_target_bytes(target: &Path) -> u64 {
    sweep_cargo_cache_dir(target, SweepMode::DryRun).map_or(0, |report| report.freed_bytes)
}

fn sweep(
    dir: &Path,
    mode: SweepMode,
    now: SystemTime,
    keep_for: Duration,
    access_times: Option<bool>,
) -> Result<SweepReport, String> {
    let _sweep_lock = match mode {
        SweepMode::Apply => Some(hold_sweep_lock(dir)?),
        SweepMode::DryRun => None,
    };
    let leftover_trash = match mode {
        SweepMode::Apply => apply::empty_trash(dir).err(),
        SweepMode::DryRun => None,
    };
    let access_times = access_times.unwrap_or_else(|| plan::records_access_times(dir, now));
    let plan = plan::plan(dir, now, keep_for, access_times)?;
    let mut report = match mode {
        SweepMode::DryRun => plan.dry_run_report(),
        SweepMode::Apply => apply::apply(plan),
    };
    report.failures.extend(leftover_trash);
    Ok(report)
}

fn hold_sweep_lock(dir: &Path) -> Result<fs::File, String> {
    let path = dir.join(SWEEP_LOCK_FILE);
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| format!("{} ({error})", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(format!(
            "another cargo cache sweep is running in {}",
            dir.display()
        )),
        Err(std::fs::TryLockError::Error(error)) => Err(format!("{} ({error})", path.display())),
    }
}

fn hold_build_locks(profile: &Path) -> Result<Vec<fs::File>, String> {
    let mut held = Vec::new();
    for name in BUILD_LOCK_FILES {
        let path = profile.join(name);
        let file = match fs::OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{} ({error})", path.display())),
        };
        match file.try_lock() {
            Ok(()) => held.push(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(format!(
                    "cargo is building in {}; prune skipped",
                    profile.display()
                ))
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(format!("{} ({error})", path.display()))
            }
        }
    }
    Ok(held)
}

fn is_building_in_root(root: &Path) -> bool {
    root.is_dir()
        && PROFILE_DIRS
            .iter()
            .any(|profile| hold_build_locks(&root.join(profile)).is_err())
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

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);
    const HOUR: Duration = Duration::from_secs(60 * 60);

    struct Clock {
        now: SystemTime,
    }

    impl Clock {
        fn new() -> Self {
            Self {
                now: SystemTime::now() + 30 * DAY,
            }
        }

        fn fresh(&self) -> SystemTime {
            self.now - HOUR
        }

        fn old(&self) -> SystemTime {
            self.now - 10 * DAY
        }

        fn sweep(&self, dir: &Path, mode: SweepMode, access_times: bool) -> SweepReport {
            sweep(dir, mode, self.now, KEEP_UNUSED_FOR, Some(access_times)).expect("sweep")
        }
    }

    fn write_at(path: &Path, content: &[u8], time: SystemTime) {
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(path, content).expect("write");
        fs::File::options()
            .write(true)
            .open(path)
            .expect("open")
            .set_times(fs::FileTimes::new().set_accessed(time).set_modified(time))
            .expect("times");
    }

    fn hash_file_content(fingerprint: u64) -> String {
        fingerprint
            .to_le_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    struct UnitFiles {
        fingerprint: PathBuf,
        rlib: PathBuf,
        dep_info: PathBuf,
    }

    impl UnitFiles {
        fn all_exist(&self) -> bool {
            self.fingerprint.exists() && self.rlib.exists() && self.dep_info.exists()
        }

        fn none_exist(&self) -> bool {
            !self.fingerprint.exists() && !self.rlib.exists() && !self.dep_info.exists()
        }
    }

    fn unit(
        profile: &Path,
        package: &str,
        hash: &str,
        fingerprint: u64,
        dependencies: &[u64],
        used: SystemTime,
    ) -> UnitFiles {
        let dir = profile
            .join(".fingerprint")
            .join(format!("{package}-{hash}"));
        let deps: Vec<String> = dependencies
            .iter()
            .map(|dependency| format!("[7,\"dep\",false,{dependency}]"))
            .collect();
        write_at(
            &dir.join(format!("lib-{package}.json")),
            format!("{{\"rustc\":1,\"deps\":[{}]}}", deps.join(",")).as_bytes(),
            used,
        );
        write_at(
            &dir.join(format!("lib-{package}")),
            hash_file_content(fingerprint).as_bytes(),
            used,
        );
        write_at(&dir.join(format!("dep-lib-{package}")), b"", used);
        let rlib = profile
            .join("deps")
            .join(format!("lib{package}-{hash}.rlib"));
        write_at(&rlib, b"rlib bytes", used);
        let dep_info = profile.join("deps").join(format!("{package}-{hash}.d"));
        write_at(&dep_info, b"dep info", used);
        UnitFiles {
            fingerprint: dir,
            rlib,
            dep_info,
        }
    }

    fn base36(mut value: u64) -> String {
        const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
        let mut encoded = Vec::new();
        loop {
            encoded.push(DIGITS[(value % 36) as usize]);
            value /= 36;
            if value == 0 {
                break;
            }
        }
        encoded.reverse();
        String::from_utf8(encoded).expect("ascii")
    }

    struct SessionFiles {
        dir: PathBuf,
        lock: PathBuf,
    }

    fn session(crate_dir: &Path, started: SystemTime, suffix: &str) -> SessionFiles {
        let micros = started
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("after epoch")
            .as_micros();
        let stem = format!(
            "s-{}-1abc",
            base36(u64::try_from(micros).expect("micros fit u64"))
        );
        let dir = crate_dir.join(format!("{stem}-{suffix}"));
        write_at(&dir.join("dep-graph.bin"), b"graph", started);
        let lock = crate_dir.join(format!("{stem}.lock"));
        write_at(&lock, b"", started);
        SessionFiles { dir, lock }
    }

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
    fn build_dir_counts_only_when_cargo_separates_it_from_target() {
        let cases = [
            (
                serde_json::json!({"target_directory": "/r/target", "build_directory": "/g/.cargo-build"}),
                Some(PathBuf::from("/g/.cargo-build")),
            ),
            (
                serde_json::json!({"target_directory": "/r/target", "build_directory": "/r/target"}),
                None,
            ),
            (serde_json::json!({"target_directory": "/r/target"}), None),
        ];
        for (metadata, expected) in cases {
            assert_eq!(separate_build_dir(&metadata), expected, "{metadata}");
        }
    }

    #[test]
    fn sweep_keeps_used_units_and_what_they_build_on_and_drops_the_rest() {
        let clock = Clock::new();
        let root = tempfile::tempdir().expect("tempdir");
        let profile = root.path().join("debug");
        let used = unit(&profile, "app", "aaaaaaaaaaaaaaaa", 1, &[2], clock.fresh());
        let dependency = unit(&profile, "serde", "bbbbbbbbbbbbbbbb", 2, &[], clock.old());
        let unused = unit(&profile, "syn", "cccccccccccccccc", 3, &[], clock.old());
        let orphan = profile.join("deps").join("liborphan-dddddddddddddddd.rlib");
        write_at(&orphan, b"orphan", clock.fresh());
        let stray = profile.join("deps").join("rmetaXyZ123");
        write_at(&stray, b"temp", clock.old());

        let planned = clock.sweep(root.path(), SweepMode::DryRun, true);

        assert_eq!(planned.units.count, 2, "the unused unit and the orphan");
        assert!(planned.freed_bytes > 0);
        assert!(
            unused.all_exist() && orphan.exists(),
            "a dry run removes nothing"
        );

        let applied = clock.sweep(root.path(), SweepMode::Apply, true);

        assert_eq!(applied.units, planned.units);
        assert_eq!(applied.freed_bytes, planned.freed_bytes);
        assert!(applied.failures.is_empty(), "{:?}", applied.failures);
        assert!(
            used.all_exist(),
            "a unit cargo read within the window stays"
        );
        assert!(
            dependency.all_exist(),
            "an old unit that a used unit builds on stays"
        );
        assert!(unused.none_exist(), "an unused unit goes as a whole");
        assert!(!orphan.exists(), "artifacts without a fingerprint go");
        assert!(stray.exists(), "files that name no unit are left alone");
        assert!(!root.path().join(".qol-sweep-trash").exists());
        assert_eq!(
            clock
                .sweep(root.path(), SweepMode::DryRun, true)
                .units
                .count,
            0
        );
    }

    #[test]
    fn without_access_times_only_unusable_caches_go() {
        let clock = Clock::new();
        let root = tempfile::tempdir().expect("tempdir");
        let profile = root.path().join("debug");
        let old = unit(&profile, "syn", "cccccccccccccccc", 3, &[], clock.old());
        let orphan = profile.join("deps").join("liborphan-dddddddddddddddd.rlib");
        write_at(&orphan, b"orphan", clock.old());
        let idle = profile.join("incremental").join("syn-1x");
        let idle_session = session(&idle, clock.old(), "svh1");
        let replaced = session(&idle, clock.old() - DAY, "svh0");
        let stale_root = root.path().join("tmp").join("leftover");
        write_at(&stale_root, b"x", clock.old());

        let report = clock.sweep(root.path(), SweepMode::Apply, false);

        assert!(!report.access_times);
        assert!(
            old.all_exist(),
            "unit age is unknowable without access times"
        );
        assert!(idle_session.dir.exists(), "so is crate cache age");
        assert!(stale_root.exists(), "and cache root age");
        assert!(!orphan.exists());
        assert!(!replaced.dir.exists() && !replaced.lock.exists());
        assert_eq!(report.units.count, 1);
        assert_eq!(report.superseded_sessions.count, 1);
        assert_eq!(report.idle_crate_caches.count, 0);
    }

    #[test]
    fn incremental_sweep_keeps_the_session_rustc_resumes_from() {
        let clock = Clock::new();
        let root = tempfile::tempdir().expect("tempdir");
        let incremental = root.path().join("debug").join("incremental");
        fs::create_dir_all(root.path().join("debug").join(".fingerprint")).expect("profile");
        let hot = incremental.join("app-1a");
        let newest = session(&hot, clock.fresh(), "svh2");
        let replaced = session(&hot, clock.old(), "svh1");
        let idle = incremental.join("old_crate-2b");
        session(&idle, clock.old(), "svh3");
        let busy = incremental.join("busy-3c");
        let busy_finalized = session(&busy, clock.old(), "svh4");
        let working = session(&busy, clock.old(), "working");
        let working_lock = fs::File::open(&working.lock).expect("working lock");
        working_lock.lock().expect("rustc holds the lock");
        let abandoned = incremental.join("crashed-4d");
        let abandoned_finalized = session(&abandoned, clock.old(), "svh6");
        session(&abandoned, clock.old(), "working");

        let report = clock.sweep(root.path(), SweepMode::Apply, true);

        assert!(newest.dir.exists() && newest.lock.exists());
        assert!(!replaced.dir.exists(), "rustc never loads an older session");
        assert!(!replaced.lock.exists());
        assert!(
            !idle.exists(),
            "a crate cache idle past the window goes whole"
        );
        assert!(
            busy_finalized.dir.exists(),
            "a crate cache with a working session is left to rustc"
        );
        assert!(
            !abandoned_finalized.dir.exists(),
            "a working session nobody holds a lock on is a crash leftover"
        );
        assert_eq!(report.superseded_sessions.count, 1);
        assert_eq!(report.idle_crate_caches.count, 2);
    }

    #[cfg(unix)]
    #[test]
    fn linked_outputs_root_their_units_and_dependencies() {
        let clock = Clock::new();
        let root = tempfile::tempdir().expect("tempdir");
        let build_dir = root.path().join("build-dir");
        let profile = build_dir.join("debug");
        let bin = unit(&profile, "tray", "aaaaaaaaaaaaaaaa", 4, &[5], clock.old());
        let dependency = unit(&profile, "serde", "bbbbbbbbbbbbbbbb", 5, &[], clock.old());
        let unrelated = unit(&profile, "syn", "cccccccccccccccc", 6, &[], clock.old());
        let uplifted = root.path().join("target").join("debug").join("tray");
        fs::create_dir_all(uplifted.parent().expect("parent")).expect("target dir");
        fs::hard_link(&bin.rlib, &uplifted).expect("uplift");
        let live = unit(&profile, "used", "eeeeeeeeeeeeeeee", 7, &[], clock.fresh());
        let resumable = profile.join("incremental").join("used-4d");
        let resumable_session = session(&resumable, clock.old(), "svh5");
        fs::hard_link(&live.rlib, resumable_session.dir.join("metadata.rmeta"))
            .expect("session link");

        let report = clock.sweep(&build_dir, SweepMode::Apply, true);

        assert!(bin.all_exist(), "a unit a target dir links is a root");
        assert!(dependency.all_exist(), "a root keeps what it builds on");
        assert!(unrelated.none_exist());
        assert!(
            resumable_session.dir.exists(),
            "an idle crate cache whose output a live unit holds stays"
        );
        assert_eq!(report.units.count, 1);
        assert_eq!(report.idle_crate_caches.count, 0);
    }

    #[cfg(unix)]
    #[test]
    fn freed_bytes_count_a_hard_linked_file_once_and_only_when_every_link_goes() {
        let clock = Clock::new();
        let root = tempfile::tempdir().expect("tempdir");
        let profile = root.path().join("debug");
        let first = unit(&profile, "one", "aaaaaaaaaaaaaaaa", 1, &[], clock.old());
        let second_rlib = profile.join("deps").join("libtwo-bbbbbbbbbbbbbbbb.rlib");
        fs::hard_link(&first.rlib, &second_rlib).expect("shared rlib");
        let kept = profile.join("kept-link.rlib");
        let third_rlib = profile.join("deps").join("libthree-cccccccccccccccc.rlib");
        write_at(&third_rlib, b"0123456789", clock.old());
        fs::hard_link(&third_rlib, &kept).expect("outside link");

        let report = clock.sweep(root.path(), SweepMode::DryRun, true);

        let first_bytes: u64 = [&first.fingerprint, &first.rlib, &first.dep_info]
            .iter()
            .map(|path| path_bytes(path))
            .sum();
        assert_eq!(report.units.count, 3);
        assert_eq!(
            report.freed_bytes, first_bytes,
            "the shared rlib counts once; the rlib a link outside keeps frees nothing"
        );
    }

    #[test]
    fn stale_roots_go_while_protected_and_recent_roots_stay() {
        let clock = Clock::new();
        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path();
        let stale = target.join("tmp").join("leftover");
        write_at(&stale, b"x", clock.old());
        let recent = target.join("doc").join("index.html");
        write_at(&recent, b"x", clock.fresh());
        let read_lately = target.join("qol-dev-bundle").join("debug").join("qol-tray");
        write_at(&read_lately, b"x", clock.old());
        fs::File::options()
            .write(true)
            .open(&read_lately)
            .expect("open")
            .set_times(fs::FileTimes::new().set_accessed(clock.fresh()))
            .expect("read time");
        for protected in [
            "qol-env/lane/qol-tray",
            "qol-check/lint-cache/data",
            "qol-lint/cache.bin",
            "qol-dev/runtime/gen/qol-tray",
            "sherpa-onnx-prebuilt/model.bin",
            "debug/qol-tray",
            "release/qol-tray",
            "CACHEDIR.TAG",
            ".rustc_info.json",
        ] {
            write_at(&target.join(protected), b"x", clock.old());
        }
        let cross = target.join("x86_64-pc-windows-gnu").join("debug");
        let cross_unit = unit(&cross, "syn", "cccccccccccccccc", 3, &[], clock.old());
        let cross_used = unit(&cross, "app", "aaaaaaaaaaaaaaaa", 1, &[], clock.fresh());

        let report = clock.sweep(target, SweepMode::Apply, true);

        assert!(
            !target.join("tmp").exists(),
            "an idle cache root goes whole"
        );
        assert!(recent.exists(), "a root written within the window stays");
        assert!(
            read_lately.exists(),
            "so does a root read within the window"
        );
        for protected in [
            "qol-env/lane/qol-tray",
            "qol-check/lint-cache/data",
            "qol-lint/cache.bin",
            "qol-dev/runtime/gen/qol-tray",
            "sherpa-onnx-prebuilt/model.bin",
            "debug/qol-tray",
            "release/qol-tray",
            "CACHEDIR.TAG",
            ".rustc_info.json",
        ] {
            assert!(target.join(protected).exists(), "{protected}");
        }
        assert!(
            cross_unit.none_exist(),
            "a cross target sweeps unit by unit"
        );
        assert!(cross_used.all_exist(), "instead of going whole");
        assert_eq!(report.stale_roots.count, 1);
    }

    #[test]
    fn a_profile_cargo_is_building_in_is_skipped() {
        let clock = Clock::new();
        let root = tempfile::tempdir().expect("tempdir");
        let profile = root.path().join("debug");
        let unused = unit(&profile, "syn", "cccccccccccccccc", 3, &[], clock.old());
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(profile.join(".cargo-build-lock"))
            .expect("lock file");
        lock.lock().expect("acquire lock");

        let report = clock.sweep(root.path(), SweepMode::Apply, true);

        assert!(
            unused.all_exist(),
            "nothing goes while cargo holds the lock"
        );
        assert_eq!(report.units.count, 0);
        assert!(
            report
                .skipped
                .iter()
                .any(|reason| reason.contains("cargo is building")),
            "{:?}",
            report.skipped
        );
        let error = prune_cargo_target_dir(root.path()).expect_err("skips surface as errors");
        assert!(error.contains("cargo is building"), "error: {error}");
    }

    #[test]
    fn one_sweep_runs_at_a_time() {
        let root = tempfile::tempdir().expect("tempdir");
        let _held = hold_sweep_lock(root.path()).expect("first sweep");

        let error = sweep(
            root.path(),
            SweepMode::Apply,
            SystemTime::now(),
            KEEP_UNUSED_FOR,
            Some(true),
        )
        .expect_err("second sweep");

        assert!(error.contains("another cargo cache sweep"), "{error}");
    }

    #[test]
    fn a_leftover_trash_dir_is_emptied_before_the_next_sweep() {
        let clock = Clock::new();
        let root = tempfile::tempdir().expect("tempdir");
        write_at(&root.path().join("CACHEDIR.TAG"), b"x", clock.old());
        write_at(
            &root
                .path()
                .join(".qol-sweep-trash")
                .join("7")
                .join("lib.rlib"),
            b"x",
            clock.old(),
        );

        let report = clock.sweep(root.path(), SweepMode::Apply, true);

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert!(!root.path().join(".qol-sweep-trash").exists());
    }

    #[test]
    fn a_dir_that_is_not_a_cargo_cache_is_refused_untouched() {
        let clock = Clock::new();
        let root = tempfile::tempdir().expect("tempdir");
        let work = root.path().join("work");
        let repo = work.join("some-repo");
        write_at(&repo.join("src").join("main.rs"), b"fn main() {}", clock.old());
        let profile = root.path().join("target").join("debug");
        unit(&profile, "syn", "cccccccccccccccc", 3, &[], clock.old());

        for dir in [&work, &profile] {
            for mode in [SweepMode::DryRun, SweepMode::Apply] {
                sweep(dir, mode, clock.now, KEEP_UNUSED_FOR, Some(true)).expect_err("refused");
            }
        }

        assert!(repo.join("src").join("main.rs").exists());
        assert!(profile.join("deps").exists());
    }

    #[test]
    fn the_access_time_probe_leaves_nothing_behind() {
        let root = tempfile::tempdir().expect("tempdir");

        plan::records_access_times(root.path(), SystemTime::now());

        assert_eq!(fs::read_dir(root.path()).expect("read dir").count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn sweep_continues_past_unremovable_entries() {
        use std::os::unix::fs::PermissionsExt;

        let clock = Clock::new();
        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path();
        write_at(&target.join("CACHEDIR.TAG"), b"x", clock.old());
        let locked = target.join("cargo-timings");
        write_at(&locked.join("held.rlib"), b"x", clock.old());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("lock perms");
        let removable = target.join("sandbox");
        write_at(&removable.join("f"), b"x", clock.old());

        let report = clock.sweep(target, SweepMode::Apply, true);

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("unlock perms");
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.contains("cargo-timings")),
            "{:?}",
            report.failures
        );
        assert!(
            !removable.exists(),
            "one unremovable entry must not stop the rest of the sweep"
        );
    }
}
