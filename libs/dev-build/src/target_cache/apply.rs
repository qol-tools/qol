use super::files::last_touched;
use super::incremental::{idle_since, newest_finalized, session_names};
use super::plan::{tally_into, Guard, Plan, Removal};
use super::units::{fingerprinted_hashes, last_use};
use super::{hold_build_locks, is_building_in_root, try_remove_target_path, Pacer, SweepReport};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const TRASH_DIR: &str = ".qol-sweep-trash";

pub(super) fn empty_trash(dir: &Path) -> Result<(), String> {
    let trash = dir.join(TRASH_DIR);
    match fs::remove_dir_all(&trash) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{} ({error})", trash.display())),
    }
}

pub(super) fn apply(plan: Plan) -> SweepReport {
    let mut report = plan.report(false);
    let mut trash = Trash::new(plan.dir.join(TRASH_DIR));
    let mut removed: Vec<Removal> = Vec::new();
    for profile in plan.profiles {
        let _locks = match hold_build_locks(&profile.path) {
            Ok(locks) => locks,
            Err(reason) => {
                report.skipped.push(reason);
                continue;
            }
        };
        let fingerprinted = fingerprinted_hashes(&profile.path);
        for removal in profile.removals {
            if still_unused(
                &removal.guard,
                &fingerprinted,
                plan.cutoff,
                plan.access_times,
            ) && trash.take(&removal.paths, &mut report.failures)
            {
                removed.push(removal);
            }
        }
    }
    for removal in plan.stale_roots {
        let Some(path) = removal.paths.first() else {
            continue;
        };
        if is_building_in_root(path) {
            report
                .skipped
                .push(format!("cargo is building in {}; kept", path.display()));
            continue;
        }
        if last_touched(path, &mut Pacer::new()) < plan.cutoff
            && trash.take(&removal.paths, &mut report.failures)
        {
            removed.push(removal);
        }
    }
    if let Err(failure) = trash.empty() {
        report.failures.push(failure);
    }
    tally_into(&mut report, &removed);
    report
}

fn still_unused(
    guard: &Guard,
    fingerprinted: &HashSet<String>,
    cutoff: SystemTime,
    access_times: bool,
) -> bool {
    match guard {
        Guard::Unit {
            hash,
            fingerprint: None,
        } => !fingerprinted.contains(hash),
        Guard::Unit {
            fingerprint: Some(fingerprint),
            ..
        } => last_use(fingerprint, access_times) < cutoff,
        Guard::Session { crate_dir, started } => {
            newest_finalized(&session_names(crate_dir)).is_some_and(|newest| newest > *started)
        }
        Guard::CrateCache { crate_dir } => idle_since(&session_names(crate_dir), cutoff),
        Guard::Root => true,
    }
}

struct Trash {
    path: PathBuf,
    next: u64,
    created: bool,
}

impl Trash {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            next: 0,
            created: false,
        }
    }

    fn take(&mut self, paths: &[PathBuf], failures: &mut Vec<String>) -> bool {
        let before = failures.len();
        for path in paths {
            if fs::symlink_metadata(path).is_err() {
                continue;
            }
            if !self.created {
                self.created = fs::create_dir_all(&self.path).is_ok();
            }
            let destination = self.path.join(self.next.to_string());
            self.next += 1;
            if self.created && fs::rename(path, &destination).is_ok() {
                continue;
            }
            try_remove_target_path(path, failures);
        }
        failures.len() == before
    }

    fn empty(&self) -> Result<(), String> {
        if !self.created {
            return Ok(());
        }
        fs::remove_dir_all(&self.path).map_err(|error| format!("{} ({error})", self.path.display()))
    }
}
