use super::files::{collect_files, last_touched, read_entries, FileStat};
use super::incremental::{self, CrateCache};
use super::units::{self, UnitScan, FINGERPRINT_DIR};
use super::{is_protected_target_root, Pacer, SweepReport, Tally, PROFILE_DIRS};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const CACHE_MARKERS: [&str; 2] = ["CACHEDIR.TAG", ".rustc_info.json"];

pub(super) struct Plan {
    pub(super) dir: PathBuf,
    pub(super) access_times: bool,
    pub(super) keep_for: Duration,
    pub(super) cutoff: SystemTime,
    pub(super) profiles: Vec<ProfilePlan>,
    pub(super) stale_roots: Vec<Removal>,
}

pub(super) struct ProfilePlan {
    pub(super) path: PathBuf,
    pub(super) removals: Vec<Removal>,
}

pub(super) struct Removal {
    pub(super) kind: RemovalKind,
    pub(super) paths: Vec<PathBuf>,
    pub(super) files: Vec<FileStat>,
    pub(super) guard: Guard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RemovalKind {
    Unit,
    SupersededSession,
    IdleCrateCache,
    StaleRoot,
}

pub(super) enum Guard {
    Unit {
        hash: String,
        fingerprint: Option<PathBuf>,
    },
    Session {
        crate_dir: PathBuf,
        started: SystemTime,
    },
    CrateCache {
        crate_dir: PathBuf,
    },
    Root,
}

impl Plan {
    pub(super) fn report(&self, dry_run: bool) -> SweepReport {
        SweepReport {
            dir: self.dir.clone(),
            dry_run,
            access_times: self.access_times,
            keep_unused_days: self.keep_for.as_secs() / DAY.as_secs(),
            freed_bytes: 0,
            units: Tally::default(),
            superseded_sessions: Tally::default(),
            idle_crate_caches: Tally::default(),
            stale_roots: Tally::default(),
            skipped: Vec::new(),
            failures: Vec::new(),
        }
    }

    pub(super) fn dry_run_report(&self) -> SweepReport {
        let mut report = self.report(true);
        let removals = self
            .profiles
            .iter()
            .flat_map(|profile| &profile.removals)
            .chain(&self.stale_roots);
        tally_into(&mut report, removals);
        report
    }
}

pub(super) fn plan(
    dir: &Path,
    now: SystemTime,
    keep_for: Duration,
    access_times: bool,
) -> Result<Plan, String> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)
        .map_err(|error| format!("failed to read cargo cache dir {}: {error}", dir.display()))?
        .flatten()
        .collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    ensure_cargo_cache(dir, &entries)?;
    let cutoff = now.checked_sub(keep_for).unwrap_or(SystemTime::UNIX_EPOCH);
    let unit_cutoff = access_times.then_some(cutoff);
    let mut pacer = Pacer::new();
    let mut plan = Plan {
        dir: dir.to_path_buf(),
        access_times,
        keep_for,
        cutoff,
        profiles: Vec::new(),
        stale_roots: Vec::new(),
    };
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let protected = is_protected_target_root(&name);
        if is_profile_dir(&path) && (!protected || PROFILE_DIRS.contains(&name.as_str())) {
            plan.profiles
                .push(plan_profile(&path, unit_cutoff, &mut pacer));
            continue;
        }
        if protected {
            continue;
        }
        let nested = child_profile_dirs(&path);
        if !nested.is_empty() {
            for profile in nested {
                plan.profiles
                    .push(plan_profile(&profile, unit_cutoff, &mut pacer));
            }
            continue;
        }
        if access_times && last_touched(&path, &mut pacer) < cutoff {
            let mut files = Vec::new();
            collect_files(&path, &mut files, &mut pacer);
            plan.stale_roots.push(Removal {
                kind: RemovalKind::StaleRoot,
                paths: vec![path],
                files,
                guard: Guard::Root,
            });
        }
    }
    Ok(plan)
}

fn ensure_cargo_cache(dir: &Path, entries: &[fs::DirEntry]) -> Result<(), String> {
    if is_profile_dir(dir) {
        return Err(format!(
            "{} is a cargo profile dir, not a cargo cache dir; pass its parent",
            dir.display()
        ));
    }
    let marked = entries
        .iter()
        .any(|entry| CACHE_MARKERS.contains(&entry.file_name().to_string_lossy().as_ref()));
    let has_profile = || {
        entries.iter().any(|entry| {
            let path = entry.path();
            is_profile_dir(&path) || !child_profile_dirs(&path).is_empty()
        })
    };
    if marked || has_profile() {
        return Ok(());
    }
    Err(format!(
        "{} does not look like a cargo cache dir (no CACHEDIR.TAG, .rustc_info.json or profile dir); nothing pruned",
        dir.display()
    ))
}

pub(super) fn records_access_times(dir: &Path, now: SystemTime) -> bool {
    let probe = dir.join(format!(".qol-access-probe-{}", std::process::id()));
    let recorded = probe_access_time(&probe, now).unwrap_or(false);
    let _ = fs::remove_file(&probe);
    recorded
}

fn probe_access_time(probe: &Path, now: SystemTime) -> std::io::Result<bool> {
    let accessed = now.checked_sub(2 * DAY).unwrap_or(SystemTime::UNIX_EPOCH);
    let modified = accessed.checked_sub(DAY).unwrap_or(SystemTime::UNIX_EPOCH);
    let mut file = fs::File::create(probe)?;
    file.write_all(b"probe")?;
    file.set_times(
        fs::FileTimes::new()
            .set_accessed(accessed)
            .set_modified(modified),
    )?;
    drop(file);
    fs::read(probe)?;
    Ok(fs::metadata(probe)?.accessed()? > accessed)
}

fn plan_profile(profile: &Path, cutoff: Option<SystemTime>, pacer: &mut Pacer) -> ProfilePlan {
    let scan = units::scan(profile, cutoff.is_some(), pacer);
    let crates = incremental::scan(profile, pacer);
    let live = units::live_units(&scan, &root_units(&scan, &crates), cutoff);
    let live_files: HashSet<(u64, u64)> = live
        .iter()
        .filter_map(|hash| scan.units.get(hash))
        .flat_map(|unit| &unit.files)
        .filter_map(|file| file.identity)
        .collect();
    let mut dead: Vec<(String, units::Unit)> = scan
        .units
        .into_iter()
        .filter(|(hash, _)| !live.contains(hash))
        .collect();
    dead.sort_by(|left, right| left.0.cmp(&right.0));
    let mut removals: Vec<Removal> = dead
        .into_iter()
        .map(|(hash, unit)| Removal {
            kind: RemovalKind::Unit,
            paths: unit.paths,
            files: unit.files,
            guard: Guard::Unit {
                hash,
                fingerprint: unit.fingerprint,
            },
        })
        .collect();
    for cache in &crates {
        if cutoff.is_some_and(|cutoff| cache.idle_since(cutoff, &live_files)) {
            removals.push(Removal {
                kind: RemovalKind::IdleCrateCache,
                paths: vec![cache.path.clone()],
                files: cache.files().copied().collect(),
                guard: Guard::CrateCache {
                    crate_dir: cache.path.clone(),
                },
            });
            continue;
        }
        for session in cache.superseded() {
            removals.push(Removal {
                kind: RemovalKind::SupersededSession,
                paths: vec![session.path.clone(), session.lock.clone()],
                files: session.files.clone(),
                guard: Guard::Session {
                    crate_dir: cache.path.clone(),
                    started: session.name.started,
                },
            });
        }
    }
    ProfilePlan {
        path: profile.to_path_buf(),
        removals,
    }
}

fn root_units<'a>(scan: &'a UnitScan, crates: &[CrateCache]) -> HashSet<&'a str> {
    let mut internal_links: HashMap<(u64, u64), u64> = HashMap::new();
    let unit_files = scan.units.values().flat_map(|unit| &unit.files);
    for file in unit_files.chain(crates.iter().flat_map(CrateCache::files)) {
        if let Some(identity) = file.identity {
            *internal_links.entry(identity).or_default() += 1;
        }
    }
    scan.units
        .iter()
        .filter(|(_, unit)| {
            unit.files.iter().any(|file| {
                file.identity.is_some_and(|identity| {
                    file.links > internal_links.get(&identity).copied().unwrap_or(0)
                })
            })
        })
        .map(|(hash, _)| hash.as_str())
        .collect()
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

fn is_profile_dir(path: &Path) -> bool {
    is_real_dir(path) && is_real_dir(&path.join(FINGERPRINT_DIR))
}

fn child_profile_dirs(path: &Path) -> Vec<PathBuf> {
    if !is_real_dir(path) {
        return Vec::new();
    }
    let mut profiles: Vec<PathBuf> = read_entries(path)
        .map(|entry| entry.path())
        .filter(|child| is_profile_dir(child))
        .collect();
    profiles.sort();
    profiles
}

pub(super) fn tally_into<'a>(
    report: &mut SweepReport,
    removals: impl IntoIterator<Item = &'a Removal>,
) {
    let mut shared: HashMap<(u64, u64), (u64, u64, u64, RemovalKind)> = HashMap::new();
    for removal in removals {
        tally_for(report, removal.kind).count += 1;
        for file in &removal.files {
            match file.identity {
                None => add_freed(report, removal.kind, file.bytes),
                Some(identity) => {
                    shared
                        .entry(identity)
                        .or_insert((0, file.links, file.bytes, removal.kind))
                        .0 += 1;
                }
            }
        }
    }
    for (seen, links, bytes, kind) in shared.into_values() {
        if seen >= links {
            add_freed(report, kind, bytes);
        }
    }
}

fn add_freed(report: &mut SweepReport, kind: RemovalKind, bytes: u64) {
    tally_for(report, kind).bytes += bytes;
    report.freed_bytes += bytes;
}

fn tally_for(report: &mut SweepReport, kind: RemovalKind) -> &mut Tally {
    match kind {
        RemovalKind::Unit => &mut report.units,
        RemovalKind::SupersededSession => &mut report.superseded_sessions,
        RemovalKind::IdleCrateCache => &mut report.idle_crate_caches,
        RemovalKind::StaleRoot => &mut report.stale_roots,
    }
}
