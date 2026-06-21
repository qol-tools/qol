pub mod classify;
pub mod platform;

use std::path::PathBuf;

use anyhow::Result;

pub use classify::MatchKind;
pub use platform::{AppPlatform, Platform};

#[derive(Debug, Clone, serde::Serialize)]
pub struct InstalledApp {
    pub name: String,
    pub bundle_id: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct RemovalPlan {
    pub app: InstalledApp,
    pub items: Vec<Leftover>,
    pub total_bytes: u64,
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
    Platform::default()
}

pub fn installed_apps() -> Result<Vec<InstalledApp>> {
    platform().installed_apps()
}

pub fn plan(app: &InstalledApp) -> Result<RemovalPlan> {
    platform().scan(app)
}

pub fn is_protected(app: &InstalledApp) -> bool {
    platform().is_protected(app)
}

pub fn search(query: &str) -> Result<Vec<InstalledApp>> {
    Ok(filter(&installed_apps()?, query))
}

pub fn resolve_unique(query: &str) -> Result<InstalledApp> {
    pick_unique(installed_apps()?, query)
}

pub fn remove(plan: &RemovalPlan, how: Disposal) -> Result<RemovalOutcome> {
    remove_with(&platform(), plan, how)
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
    how: Disposal,
) -> Result<RemovalOutcome> {
    if plat.is_protected(&plan.app) {
        anyhow::bail!(
            "removeapp: {} is protected and cannot be removed",
            plan.app.name
        );
    }
    let paths: Vec<PathBuf> = plan.items.iter().map(|l| l.path.clone()).collect();
    plat.remove_paths(&paths, how)
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

    fn plan_for(a: InstalledApp) -> RemovalPlan {
        RemovalPlan {
            items: vec![Leftover {
                path: a.path.clone(),
                kind: LeftoverKind::AppBundle,
                size_bytes: 0,
                match_kind: MatchKind::Exact,
            }],
            app: a,
            total_bytes: 0,
        }
    }

    #[derive(Default)]
    struct FakePlat {
        protected: bool,
        removed: RefCell<Vec<PathBuf>>,
    }

    impl AppPlatform for FakePlat {
        fn installed_apps(&self) -> Result<Vec<InstalledApp>> {
            Ok(vec![])
        }
        fn scan(&self, app: &InstalledApp) -> Result<RemovalPlan> {
            Ok(plan_for(app.clone()))
        }
        fn remove_paths(&self, paths: &[PathBuf], _how: Disposal) -> Result<RemovalOutcome> {
            self.removed.borrow_mut().extend_from_slice(paths);
            Ok(RemovalOutcome {
                removed: paths.to_vec(),
                failed: vec![],
                freed_bytes: 0,
            })
        }
        fn is_protected(&self, _app: &InstalledApp) -> bool {
            self.protected
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
        let p = plan_for(app("Defender", "com.microsoft.wdav"));
        assert!(remove_with(&fake, &p, Disposal::Trash).is_err());
        assert!(fake.removed.borrow().is_empty(), "no paths touched");
    }

    #[test]
    fn remove_unprotected_removes_paths() {
        let fake = FakePlat::default();
        let p = plan_for(app("Foo", "com.acme.foo"));
        let out = remove_with(&fake, &p, Disposal::Delete).unwrap();
        assert_eq!(out.removed.len(), 1);
        assert!(!fake.removed.borrow().is_empty());
    }
}
