pub mod classify;
pub mod guards;
pub mod platform;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;

pub use classify::MatchKind;
pub use guards::{CaskIndex, CaskStatus, CaskToken, Guards};
pub use platform::{AppPlatform, Platform};

#[derive(Debug, Clone, serde::Serialize)]
pub struct InstalledApp {
    pub name: String,
    pub bundle_id: Option<String>,
    pub path: PathBuf,
}

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
            Ok(m) => IdentitySnapshot {
                exists: true,
                is_symlink: m.file_type().is_symlink(),
                is_dir: m.is_dir(),
                is_file: m.is_file(),
                dev: metadata_dev(&m),
                ino: metadata_ino(&m),
                file_name,
                canonical_parent,
                ancestor_symlink,
            },
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

#[cfg(unix)]
fn metadata_dev(meta: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.dev())
}

#[cfg(not(unix))]
fn metadata_dev(_meta: &std::fs::Metadata) -> Option<u64> {
    None
}

#[cfg(unix)]
fn metadata_ino(meta: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.ino())
}

#[cfg(not(unix))]
fn metadata_ino(_meta: &std::fs::Metadata) -> Option<u64> {
    None
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

pub fn cask_index() -> CaskIndex {
    platform().cask_index()
}

pub fn guards(app: &InstalledApp, inventory: &[InstalledApp]) -> Guards {
    guards_with(app, inventory, &cask_index())
}

pub fn guards_with(app: &InstalledApp, inventory: &[InstalledApp], index: &CaskIndex) -> Guards {
    Guards {
        running: platform().is_running(app),
        cask: classify_cask(app, inventory, index),
    }
}

fn classify_cask(app: &InstalledApp, inventory: &[InstalledApp], index: &CaskIndex) -> CaskStatus {
    let base = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let inv: Vec<String> = inventory.iter().map(|a| base(&a.path)).collect();
    index.classify(&base(&app.path), &inv)
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

pub fn brew_uninstall(token: &CaskToken) -> Result<()> {
    platform().brew_uninstall(token)
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
    cask: &CaskStatus,
) -> Result<RemovalOutcome> {
    remove_with(&platform(), plan, requested, cask)
}

pub fn remove_after_brew(
    plan: &RemovalPlan,
    requested: Disposal,
    cask: &CaskStatus,
    brew_handled_bundle: bool,
) -> Result<RemovalOutcome> {
    remove_after_brew_with(&platform(), plan, requested, cask, brew_handled_bundle)
}

fn remove_after_brew_with(
    plat: &impl AppPlatform,
    plan: &RemovalPlan,
    requested: Disposal,
    cask: &CaskStatus,
    brew_handled_bundle: bool,
) -> Result<RemovalOutcome> {
    if !brew_handled_bundle {
        return remove_with(plat, plan, requested, cask);
    }
    ensure_snapshot_alignment(plan)?;
    let mut items = Vec::new();
    let mut snapshots = Vec::new();
    for (item, snapshot) in plan.items.iter().zip(&plan.snapshots) {
        if item.kind == LeftoverKind::AppBundle {
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
    remove_with(plat, &sub, Disposal::Trash, cask)
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
    cask: &CaskStatus,
) -> Result<RemovalOutcome> {
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
    let cask_unavailable = matches!(cask, CaskStatus::Unavailable(_));
    let disposal_for = |item: &Leftover| {
        let bundle_override = item.kind == LeftoverKind::AppBundle && cask_unavailable;
        classify::effective_disposal(item.match_kind, requested, bundle_override)
    };

    let (bundle, rest): (Vec<&Leftover>, Vec<&Leftover>) = plan
        .items
        .iter()
        .partition(|i| i.kind == LeftoverKind::AppBundle);

    let mut outcome = RemovalOutcome::default();
    for item in &bundle {
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
        fn cask_index(&self) -> CaskIndex {
            CaskIndex::absent()
        }
        fn brew_uninstall(&self, _token: &CaskToken) -> Result<()> {
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
        assert!(remove_with(&fake, &p, Disposal::Trash, &CaskStatus::NotManaged).is_err());
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
        let out = remove_with(&fake, &p, Disposal::Trash, &CaskStatus::NotManaged).unwrap();
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
        remove_with(&fake, &p, Disposal::Delete, &CaskStatus::NotManaged).unwrap();
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
        let err = remove_with(&fake, &plan, Disposal::Trash, &CaskStatus::NotManaged).unwrap_err();
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
        let err = remove_with(&fake, &plan, Disposal::Trash, &CaskStatus::NotManaged).unwrap_err();
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

        let err = remove_with(&fake, &p, Disposal::Trash, &CaskStatus::NotManaged).unwrap_err();
        assert!(err.to_string().contains("stale plan"));
        assert!(fake.removed.borrow().is_empty(), "no mutation");
    }

    #[test]
    fn remove_after_brew_trashes_remaining_leftovers_even_when_delete_requested() {
        let fake = FakePlat::default();
        let p = plan_with(
            app("Foo", "com.acme.foo"),
            vec![leftover("/x/cache", LeftoverKind::Caches, MatchKind::Exact)],
        );
        remove_after_brew_with(
            &fake,
            &p,
            Disposal::Delete,
            &CaskStatus::Managed(CaskToken::parse("foo").unwrap()),
            true,
        )
        .unwrap();
        let recorded = fake.removed.borrow();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].1, Disposal::Trash);
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
