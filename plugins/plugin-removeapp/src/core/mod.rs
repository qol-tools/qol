pub mod classify;
pub mod guards;
pub mod platform;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;

pub use classify::MatchKind;
pub use guards::{
    Guards, ManagedPackage, PackageIndex, PackageManager, PackageScope, PackageStatus,
};
pub use platform::{AppPlatform, Platform};
pub use qol_apps::InstalledApp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum LeftoverKind {
    AppBundle,
    ApplicationSupport,
    Caches,
    Preferences,
    Containers,
    GroupContainers,
    SavedState,
    Logs,
    HttpStorages,
    WebKit,
    LaunchAgent,
    DesktopEntry,
    ApplicationBinary,
    Config,
    Data,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Leftover {
    pub path: PathBuf,
    pub kind: LeftoverKind,
    pub size_bytes: u64,
    pub match_kind: MatchKind,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct IdentitySnapshot {
    pub exists: bool,
    pub is_symlink: bool,
    pub is_dir: bool,
    pub is_file: bool,
    pub len: Option<u64>,
    pub modified_ns: Option<u128>,
    pub dev: Option<u64>,
    pub ino: Option<u64>,
    pub file_name: Option<String>,
    pub canonical_parent: Option<PathBuf>,
    pub ancestor_symlink: bool,
}

impl IdentitySnapshot {
    pub fn capture(path: &std::path::Path) -> IdentitySnapshot {
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let canonical_parent = path.parent().and_then(|p| std::fs::canonicalize(p).ok());
        let ancestor_symlink = ancestor_has_symlink(path);
        match std::fs::symlink_metadata(path) {
            Ok(m) => {
                let (dev, ino) = platform::metadata_identity(&m);
                let modified_ns = m
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos());
                IdentitySnapshot {
                    exists: true,
                    is_symlink: m.file_type().is_symlink(),
                    is_dir: m.is_dir(),
                    is_file: m.is_file(),
                    len: Some(m.len()),
                    modified_ns,
                    dev,
                    ino,
                    file_name,
                    canonical_parent,
                    ancestor_symlink,
                }
            }
            Err(_) => IdentitySnapshot {
                file_name,
                canonical_parent,
                ancestor_symlink,
                ..IdentitySnapshot::default()
            },
        }
    }

    pub fn matches(&self, path: &std::path::Path) -> bool {
        *self == IdentitySnapshot::capture(path)
    }
}

fn ancestor_has_symlink(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    parent.ancestors().any(|ancestor| {
        std::fs::symlink_metadata(ancestor).is_ok_and(|m| m.file_type().is_symlink())
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RemovalPlan {
    pub app: InstalledApp,
    pub items: Vec<Leftover>,
    pub total_bytes: u64,
    #[serde(skip)]
    pub snapshots: Vec<IdentitySnapshot>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RemovalOutcome {
    pub removed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub freed_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposal {
    Trash,
    Delete,
}

fn platform() -> Platform {
    Platform::new()
}

pub fn installed_apps() -> Result<Vec<InstalledApp>> {
    platform().installed_apps()
}

pub fn plan(app: &InstalledApp, inventory: &[InstalledApp]) -> Result<RemovalPlan> {
    platform().scan(app, inventory)
}

pub fn is_protected(app: &InstalledApp) -> bool {
    platform().is_protected(app)
}

pub fn package_index(inventory: &[InstalledApp]) -> PackageIndex {
    platform().package_index(inventory)
}

pub fn guards(app: &InstalledApp, inventory: &[InstalledApp]) -> Guards {
    guards_with(app, &package_index(inventory))
}

pub fn guards_with(app: &InstalledApp, index: &PackageIndex) -> Guards {
    Guards {
        running: platform().is_running(app),
        package: index.classify(&app.path),
    }
}

pub fn quit_app(app: &InstalledApp) -> Result<()> {
    platform().quit(app)
}

pub fn quit_and_wait(app: &InstalledApp) -> Result<()> {
    let plat = platform();
    quit_and_wait_with(&plat, app, 25, Duration::from_millis(100))
}

fn quit_and_wait_with(
    plat: &impl AppPlatform,
    app: &InstalledApp,
    attempts: usize,
    delay: Duration,
) -> Result<()> {
    plat.quit(app)?;
    for _ in 0..attempts {
        if !plat.is_running(app) {
            return Ok(());
        }
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }
    anyhow::bail!("removeapp: {} is still running", app.name)
}

pub fn is_running(app: &InstalledApp) -> bool {
    platform().is_running(app)
}

pub fn uninstall_package(plan: &RemovalPlan, package: &ManagedPackage) -> Result<()> {
    let platform = platform();
    uninstall_package_with(&platform, plan, package)
}

fn uninstall_package_with(
    platform: &impl AppPlatform,
    plan: &RemovalPlan,
    package: &ManagedPackage,
) -> Result<()> {
    validate_plan_with(platform, plan)?;
    platform.uninstall_package(&plan.app, package)
}

pub fn search(query: &str) -> Result<Vec<InstalledApp>> {
    Ok(filter(&installed_apps()?, query))
}

pub fn resolve_unique(inventory: &[InstalledApp], query: &str) -> Result<InstalledApp> {
    pick_unique(inventory.to_vec(), query)
}

pub fn remove(
    plan: &RemovalPlan,
    requested: Disposal,
    package: &PackageStatus,
) -> Result<RemovalOutcome> {
    remove_with(&platform(), plan, requested, package)
}

pub fn remove_after_package(
    plan: &RemovalPlan,
    requested: Disposal,
    package: &PackageStatus,
    package_handled_app: bool,
) -> Result<RemovalOutcome> {
    remove_after_package_with(&platform(), plan, requested, package, package_handled_app)
}

fn remove_after_package_with(
    plat: &impl AppPlatform,
    plan: &RemovalPlan,
    requested: Disposal,
    package: &PackageStatus,
    package_handled_app: bool,
) -> Result<RemovalOutcome> {
    if !package_handled_app {
        return remove_with(plat, plan, requested, package);
    }
    ensure_snapshot_alignment(plan)?;
    let mut items = Vec::new();
    let mut snapshots = Vec::new();
    for (item, snapshot) in plan.items.iter().zip(&plan.snapshots) {
        if is_primary(item.kind) {
            continue;
        }
        items.push(item.clone());
        snapshots.push(snapshot.clone());
    }
    let total_bytes = items.iter().map(|l| l.size_bytes).sum();
    let sub = RemovalPlan {
        app: plan.app.clone(),
        items,
        total_bytes,
        snapshots,
    };
    remove_with(plat, &sub, Disposal::Trash, package)
}

pub fn filter(apps: &[InstalledApp], query: &str) -> Vec<InstalledApp> {
    let q = query.to_lowercase();
    let mut out: Vec<InstalledApp> = apps
        .iter()
        .filter(|a| {
            a.name.to_lowercase().contains(&q)
                || a.bundle_id
                    .as_deref()
                    .is_some_and(|b| b.to_lowercase().contains(&q))
        })
        .cloned()
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn pick_unique(apps: Vec<InstalledApp>, query: &str) -> Result<InstalledApp> {
    let mut matches = filter(&apps, query);
    let exact_count = matches
        .iter()
        .filter(|a| a.name.eq_ignore_ascii_case(query))
        .count();
    if exact_count == 1 {
        let pos = matches
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(query))
            .expect("exact match present");
        return Ok(matches.remove(pos));
    }
    match matches.len() {
        0 => anyhow::bail!("removeapp: no app matches {query:?}"),
        1 => Ok(matches.remove(0)),
        _ => {
            let names: Vec<&str> = matches.iter().map(|a| a.name.as_str()).collect();
            anyhow::bail!("removeapp: {query:?} is ambiguous: {}", names.join(", "))
        }
    }
}

fn remove_with(
    plat: &impl AppPlatform,
    plan: &RemovalPlan,
    requested: Disposal,
    package: &PackageStatus,
) -> Result<RemovalOutcome> {
    validate_plan_with(plat, plan)?;
    let package_unavailable = matches!(package, PackageStatus::Unavailable(_));
    let disposal_for = |item: &Leftover| {
        let primary_override = is_primary(item.kind) && package_unavailable;
        classify::effective_disposal(item.match_kind, requested, primary_override)
    };

    let (primary, rest): (Vec<&Leftover>, Vec<&Leftover>) =
        plan.items.iter().partition(|i| is_primary(i.kind));

    let mut outcome = RemovalOutcome::default();
    for item in &primary {
        let res = plat.remove_items(&[(item.path.clone(), disposal_for(item))])?;
        absorb(&mut outcome, res, plan);
        if !outcome.failed.is_empty() {
            return Ok(outcome);
        }
    }
    let rest_items: Vec<(PathBuf, Disposal)> = rest
        .iter()
        .map(|i| (i.path.clone(), disposal_for(i)))
        .collect();
    let res = plat.remove_items(&rest_items)?;
    absorb(&mut outcome, res, plan);
    Ok(outcome)
}

fn validate_plan_with(plat: &impl AppPlatform, plan: &RemovalPlan) -> Result<()> {
    if plat.is_protected(&plan.app) {
        anyhow::bail!(
            "removeapp: {} is protected and cannot be removed",
            plan.app.name
        );
    }
    ensure_snapshot_alignment(plan)?;
    for (item, snap) in plan.items.iter().zip(&plan.snapshots) {
        if !snap.matches(&item.path) {
            anyhow::bail!(
                "removeapp: {} changed on disk; aborting",
                item.path.display()
            );
        }
    }
    Ok(())
}

fn is_primary(kind: LeftoverKind) -> bool {
    matches!(
        kind,
        LeftoverKind::AppBundle | LeftoverKind::DesktopEntry | LeftoverKind::ApplicationBinary
    )
}

fn ensure_snapshot_alignment(plan: &RemovalPlan) -> Result<()> {
    if plan.snapshots.len() == plan.items.len() {
        return Ok(());
    }
    anyhow::bail!("removeapp: stale plan missing identity snapshots")
}

fn absorb(acc: &mut RemovalOutcome, res: RemovalOutcome, plan: &RemovalPlan) {
    for p in res.removed {
        let size = plan
            .items
            .iter()
            .find(|i| i.path == p)
            .map(|i| i.size_bytes)
            .unwrap_or(0);
        acc.freed_bytes += size;
        acc.removed.push(p);
    }
    acc.failed.extend(res.failed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn app(name: &str, bid: &str) -> InstalledApp {
        InstalledApp {
            name: name.into(),
            bundle_id: Some(bid.into()),
            path: PathBuf::from(format!("/Applications/{name}.app")),
        }
    }

    fn leftover(path: &str, kind: LeftoverKind, mk: MatchKind) -> Leftover {
        Leftover {
            path: PathBuf::from(path),
            kind,
            size_bytes: 10,
            match_kind: mk,
        }
    }

    fn plan_with(a: InstalledApp, leftovers: Vec<Leftover>) -> RemovalPlan {
        let mut items = vec![leftover(
            a.path.to_str().unwrap(),
            LeftoverKind::AppBundle,
            MatchKind::Exact,
        )];
        items.extend(leftovers);
        let snapshots = items
            .iter()
            .map(|item| IdentitySnapshot::capture(&item.path))
            .collect();
        RemovalPlan {
            app: a,
            items,
            total_bytes: 0,
            snapshots,
        }
    }

    #[derive(Default)]
    struct FakePlat {
        protected: bool,
        fail_bundle: bool,
        running: RefCell<Vec<bool>>,
        removed: RefCell<Vec<(PathBuf, Disposal)>>,
        uninstalled: RefCell<usize>,
    }

    impl AppPlatform for FakePlat {
        fn installed_apps(&self) -> Result<Vec<InstalledApp>> {
            Ok(vec![])
        }
        fn scan(&self, app: &InstalledApp, _inv: &[InstalledApp]) -> Result<RemovalPlan> {
            Ok(plan_with(app.clone(), vec![]))
        }
        fn remove_items(&self, items: &[(PathBuf, Disposal)]) -> Result<RemovalOutcome> {
            let mut out = RemovalOutcome::default();
            for (p, d) in items {
                self.removed.borrow_mut().push((p.clone(), *d));
                if self.fail_bundle && p.to_string_lossy().ends_with(".app") {
                    out.failed.push((p.clone(), "boom".into()));
                } else {
                    out.removed.push(p.clone());
                }
            }
            Ok(out)
        }
        fn is_protected(&self, _app: &InstalledApp) -> bool {
            self.protected
        }
        fn is_running(&self, _app: &InstalledApp) -> bool {
            self.running.borrow_mut().pop().unwrap_or(false)
        }
        fn quit(&self, _app: &InstalledApp) -> Result<()> {
            Ok(())
        }
        fn package_index(&self, _inventory: &[InstalledApp]) -> PackageIndex {
            PackageIndex::absent()
        }
        fn uninstall_package(&self, _app: &InstalledApp, _package: &ManagedPackage) -> Result<()> {
            *self.uninstalled.borrow_mut() += 1;
            Ok(())
        }
    }

    #[test]
    fn pick_unique_prefers_exact_name_over_substring() {
        let apps = vec![app("Code", "com.x.code"), app("VS Code", "com.m.vscode")];
        assert!(
            pick_unique(apps.clone(), "co").is_err(),
            "bare substring matching multiple apps is ambiguous"
        );
        assert_eq!(
            pick_unique(apps, "code").unwrap().name,
            "Code",
            "exact case-insensitive name wins over substring overlap"
        );
    }

    #[test]
    fn pick_unique_errors_on_no_match() {
        assert!(pick_unique(vec![app("Foo", "com.acme.foo")], "zzz").is_err());
    }

    #[test]
    fn remove_refuses_protected_before_touching_fs() {
        let fake = FakePlat {
            protected: true,
            ..Default::default()
        };
        let p = plan_with(app("Defender", "com.microsoft.wdav"), vec![]);
        assert!(remove_with(&fake, &p, Disposal::Trash, &PackageStatus::NotManaged).is_err());
        assert!(fake.removed.borrow().is_empty(), "no paths touched");
    }

    #[test]
    fn two_phase_aborts_leftovers_when_bundle_removal_fails() {
        let fake = FakePlat {
            fail_bundle: true,
            ..Default::default()
        };
        let p = plan_with(
            app("Foo", "com.acme.foo"),
            vec![leftover("/x/cache", LeftoverKind::Caches, MatchKind::Exact)],
        );
        let out = remove_with(&fake, &p, Disposal::Trash, &PackageStatus::NotManaged).unwrap();
        assert_eq!(out.removed.len(), 0, "nothing removed");
        assert_eq!(
            fake.removed.borrow().len(),
            1,
            "only the bundle was attempted, leftovers untouched"
        );
    }

    #[test]
    fn fuzzy_leftover_is_trashed_even_when_delete_requested() {
        let fake = FakePlat::default();
        let p = plan_with(
            app("Foo", "com.acme.foo"),
            vec![leftover("/x/fuzzy", LeftoverKind::Caches, MatchKind::Fuzzy)],
        );
        remove_with(&fake, &p, Disposal::Delete, &PackageStatus::NotManaged).unwrap();
        let recorded = fake.removed.borrow();
        let fuzzy = recorded.iter().find(|(p, _)| p.ends_with("fuzzy")).unwrap();
        assert_eq!(fuzzy.1, Disposal::Trash, "fuzzy forced to Trash");
    }

    #[test]
    fn recheck_aborts_when_a_planned_path_identity_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Foo.app");
        std::fs::create_dir_all(&bundle).unwrap();
        let target = InstalledApp {
            name: "Foo".into(),
            bundle_id: Some("com.acme.foo".into()),
            path: bundle.clone(),
        };
        let snap = IdentitySnapshot::capture(&bundle);
        let plan = RemovalPlan {
            items: vec![Leftover {
                path: bundle.clone(),
                kind: LeftoverKind::AppBundle,
                size_bytes: 0,
                match_kind: MatchKind::Exact,
            }],
            app: target,
            total_bytes: 0,
            snapshots: vec![snap],
        };
        std::fs::remove_dir_all(&bundle).unwrap();
        std::fs::write(&bundle, "now a file").unwrap();

        let fake = FakePlat::default();
        let err =
            remove_with(&fake, &plan, Disposal::Trash, &PackageStatus::NotManaged).unwrap_err();
        assert!(
            err.to_string().contains("changed"),
            "aborts on identity change"
        );
        assert!(fake.removed.borrow().is_empty(), "no mutation");
    }

    #[test]
    fn recheck_aborts_when_directory_is_replaced_by_another_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Foo.app");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("old"), "old").unwrap();
        let target = InstalledApp {
            name: "Foo".into(),
            bundle_id: Some("com.acme.foo".into()),
            path: bundle.clone(),
        };
        let plan = RemovalPlan {
            items: vec![Leftover {
                path: bundle.clone(),
                kind: LeftoverKind::AppBundle,
                size_bytes: 0,
                match_kind: MatchKind::Exact,
            }],
            app: target,
            total_bytes: 0,
            snapshots: vec![IdentitySnapshot::capture(&bundle)],
        };
        let replacement = tmp.path().join("Foo.app.replacement");
        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::write(replacement.join("new"), "new").unwrap();
        std::fs::remove_dir_all(&bundle).unwrap();
        std::fs::rename(&replacement, &bundle).unwrap();

        let fake = FakePlat::default();
        let err =
            remove_with(&fake, &plan, Disposal::Trash, &PackageStatus::NotManaged).unwrap_err();
        assert!(
            err.to_string().contains("changed"),
            "aborts on same-kind identity change"
        );
        assert!(fake.removed.borrow().is_empty(), "no mutation");
    }

    #[test]
    fn remove_refuses_plans_without_complete_snapshots() {
        let fake = FakePlat::default();
        let mut p = plan_with(
            app("Foo", "com.acme.foo"),
            vec![leftover("/x/cache", LeftoverKind::Caches, MatchKind::Exact)],
        );
        p.snapshots.pop();

        let err = remove_with(&fake, &p, Disposal::Trash, &PackageStatus::NotManaged).unwrap_err();
        assert!(err.to_string().contains("stale plan"));
        assert!(fake.removed.borrow().is_empty(), "no mutation");
    }

    #[test]
    fn remove_after_package_trashes_remaining_leftovers_even_when_delete_requested() {
        let fake = FakePlat::default();
        let p = plan_with(
            app("Foo", "com.acme.foo"),
            vec![leftover("/x/cache", LeftoverKind::Caches, MatchKind::Exact)],
        );
        remove_after_package_with(
            &fake,
            &p,
            Disposal::Delete,
            &PackageStatus::Managed(
                ManagedPackage::parse(PackageManager::Homebrew, "foo", PackageScope::User).unwrap(),
            ),
            true,
        )
        .unwrap();
        let recorded = fake.removed.borrow();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].1, Disposal::Trash);
    }

    #[test]
    fn package_uninstall_revalidates_plan_before_mutating() {
        let tmp = tempfile::tempdir().unwrap();
        let launcher = tmp.path().join("app.desktop");
        std::fs::write(&launcher, "before").unwrap();
        let app = InstalledApp {
            name: "App".into(),
            bundle_id: None,
            path: launcher.clone(),
        };
        let plan = RemovalPlan {
            app,
            items: vec![Leftover {
                path: launcher.clone(),
                kind: LeftoverKind::DesktopEntry,
                size_bytes: 6,
                match_kind: MatchKind::Exact,
            }],
            total_bytes: 6,
            snapshots: vec![IdentitySnapshot::capture(&launcher)],
        };
        std::fs::remove_file(&launcher).unwrap();
        std::fs::write(&launcher, "replacement").unwrap();
        let package =
            ManagedPackage::parse(PackageManager::Apt, "fixture", PackageScope::System).unwrap();
        let fake = FakePlat::default();

        let error = uninstall_package_with(&fake, &plan, &package).unwrap_err();

        assert!(error.to_string().contains("changed"));
        assert_eq!(*fake.uninstalled.borrow(), 0, "package manager untouched");
    }

    #[test]
    fn quit_and_wait_refuses_when_app_keeps_running() {
        let fake = FakePlat {
            running: RefCell::new(vec![true, true, true]),
            ..Default::default()
        };
        let err =
            quit_and_wait_with(&fake, &app("Foo", "com.acme.foo"), 3, Duration::ZERO).unwrap_err();
        assert!(err.to_string().contains("still running"));
    }

    #[test]
    fn quit_and_wait_accepts_after_running_clears() {
        let fake = FakePlat {
            running: RefCell::new(vec![false, true]),
            ..Default::default()
        };
        quit_and_wait_with(&fake, &app("Foo", "com.acme.foo"), 3, Duration::ZERO).unwrap();
    }
}
