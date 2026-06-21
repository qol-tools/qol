use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::core::classify::{normalize_entry, owner_of};
use crate::core::guards::{CaskStatus, CaskToken};
use crate::core::{
    AppPlatform, Disposal, InstalledApp, Leftover, LeftoverKind, MatchKind, RemovalOutcome,
    RemovalPlan,
};

pub struct Platform {
    home: PathBuf,
    app_dirs: Vec<PathBuf>,
}

impl Default for Platform {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let app_dirs = vec![PathBuf::from("/Applications"), home.join("Applications")];
        Self { home, app_dirs }
    }
}

impl Platform {
    pub fn with_roots(home: PathBuf, app_dirs: Vec<PathBuf>) -> Self {
        Self { home, app_dirs }
    }

    fn library(&self) -> PathBuf {
        self.home.join("Library")
    }
}

enum KeyMode {
    Bundle,
    Hybrid,
    SharedExact,
}

fn library_dirs() -> Vec<(LeftoverKind, &'static str, KeyMode)> {
    use LeftoverKind::*;
    vec![
        (Preferences, "Preferences", KeyMode::Bundle),
        (Containers, "Containers", KeyMode::Bundle),
        (HttpStorages, "HTTPStorages", KeyMode::Bundle),
        (WebKit, "WebKit", KeyMode::Bundle),
        (SavedState, "Saved Application State", KeyMode::Bundle),
        (LaunchAgent, "LaunchAgents", KeyMode::Bundle),
        (ApplicationSupport, "Application Support", KeyMode::Hybrid),
        (Caches, "Caches", KeyMode::Hybrid),
        (Logs, "Logs", KeyMode::Hybrid),
        (GroupContainers, "Group Containers", KeyMode::SharedExact),
    ]
}

fn classify_entry(
    entry: &str,
    app: &InstalledApp,
    all_bids: &[String],
    mode: &KeyMode,
) -> Option<MatchKind> {
    let name_hit = entry.eq_ignore_ascii_case(&app.name);
    let bid_hit = app.bundle_id.as_deref().and_then(|bid| {
        let owner = owner_of(entry, all_bids)?;
        if owner != bid {
            return None;
        }
        Some(if normalize_entry(entry) == bid {
            MatchKind::Exact
        } else {
            MatchKind::Fuzzy
        })
    });
    match mode {
        KeyMode::Bundle => bid_hit,
        KeyMode::Hybrid => bid_hit.or(name_hit.then_some(MatchKind::Exact)),
        KeyMode::SharedExact => app
            .bundle_id
            .as_deref()
            .filter(|bid| normalize_entry(entry) == *bid)
            .map(|_| MatchKind::Exact),
    }
}

fn read_bundle_info(app_path: &Path) -> (Option<String>, Option<String>) {
    let info = app_path.join("Contents/Info.plist");
    let Ok(value) = plist::Value::from_file(&info) else {
        return (None, None);
    };
    let Some(dict) = value.as_dictionary() else {
        return (None, None);
    };
    let get = |k: &str| {
        dict.get(k)
            .and_then(|v| v.as_string())
            .map(|s| s.to_string())
    };
    (get("CFBundleIdentifier"), get("CFBundleName"))
}

fn path_size(path: &Path) -> u64 {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return 0,
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            total += path_size(&entry.path());
        }
    }
    total
}

const MANAGED_PREFIXES: &[&str] = &[
    "com.apple.",
    "com.microsoft.wdav",
    "com.microsoft.intune",
    "com.microsoft.autoupdate",
    "com.heimdalsecurity",
];

fn is_writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(cstr) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(cstr.as_ptr(), libc::W_OK) == 0 }
}

fn delete_path(path: &Path) -> std::result::Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let res = if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    res.map_err(|e| e.to_string())
}

fn trash_path(path: &Path) -> std::result::Result<(), String> {
    trash::delete(path).map_err(|e| e.to_string())
}

impl AppPlatform for Platform {
    fn installed_apps(&self) -> Result<Vec<InstalledApp>> {
        let mut apps = Vec::new();
        for dir in &self.app_dirs {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("app") {
                    continue;
                }
                let (bundle_id, bundle_name) = read_bundle_info(&path);
                let name = bundle_name.unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Unknown")
                        .to_string()
                });
                apps.push(InstalledApp {
                    name,
                    bundle_id,
                    path,
                });
            }
        }
        Ok(apps)
    }

    fn scan(&self, app: &InstalledApp, inventory: &[InstalledApp]) -> Result<RemovalPlan> {
        let mut all_bids: Vec<String> = inventory
            .iter()
            .filter_map(|a| a.bundle_id.clone())
            .collect();
        if let Some(bid) = &app.bundle_id {
            if !all_bids.iter().any(|b| b == bid) {
                all_bids.push(bid.clone());
            }
        }

        let mut items = vec![Leftover {
            path: app.path.clone(),
            kind: LeftoverKind::AppBundle,
            size_bytes: path_size(&app.path),
            match_kind: MatchKind::Exact,
        }];

        let lib = self.library();
        for (kind, subdir, mode) in library_dirs() {
            let Ok(entries) = fs::read_dir(lib.join(subdir)) else {
                continue;
            };
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let Some(name) = file_name.to_str() else {
                    continue;
                };
                if let Some(match_kind) = classify_entry(name, app, &all_bids, &mode) {
                    let path = entry.path();
                    let size_bytes = path_size(&path);
                    items.push(Leftover {
                        path,
                        kind,
                        size_bytes,
                        match_kind,
                    });
                }
            }
        }

        let total_bytes = items.iter().map(|l| l.size_bytes).sum();
        Ok(RemovalPlan {
            app: app.clone(),
            items,
            total_bytes,
            snapshots: vec![],
        })
    }

    fn remove_items(&self, items: &[(PathBuf, Disposal)]) -> Result<RemovalOutcome> {
        let mut outcome = RemovalOutcome::default();
        for (path, how) in items {
            let result = match how {
                Disposal::Trash => trash_path(path),
                Disposal::Delete => delete_path(path),
            };
            match result {
                Ok(()) => outcome.removed.push(path.clone()),
                Err(e) => outcome.failed.push((path.clone(), e)),
            }
        }
        Ok(outcome)
    }

    fn is_running(&self, _app: &InstalledApp) -> bool {
        false
    }

    fn quit(&self, _app: &InstalledApp) -> Result<()> {
        anyhow::bail!("removeapp: quit not yet implemented")
    }

    fn cask_status(&self, _app: &InstalledApp, _inventory: &[InstalledApp]) -> CaskStatus {
        CaskStatus::NotManaged
    }

    fn brew_uninstall(&self, _token: &CaskToken) -> Result<()> {
        anyhow::bail!("removeapp: brew_uninstall not yet implemented")
    }

    fn is_protected(&self, app: &InstalledApp) -> bool {
        let path = &app.path;
        if path.starts_with("/System") || path.starts_with("/Library/Apple") {
            return true;
        }
        if let Some(bid) = &app.bundle_id {
            if MANAGED_PREFIXES.iter().any(|p| bid.starts_with(p)) {
                return true;
            }
        }
        path.exists() && !is_writable(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO_PLIST_FOO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.acme.foo</string>
<key>CFBundleName</key><string>Foo</string>
</dict></plist>"#;

    fn write(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn scan_collects_bundle_and_present_library_leftovers() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let apps = tmp.path().join("Applications");
        let bundle = apps.join("Foo.app");
        write(&bundle.join("Contents/Info.plist"), INFO_PLIST_FOO);
        write(&home.join("Library/Caches/com.acme.foo/blob"), "xxxx");
        write(&home.join("Library/Preferences/com.acme.foo.plist"), "yy");

        let plat = Platform::with_roots(home.clone(), vec![apps.clone()]);
        let inventory = plat.installed_apps().unwrap();
        let app = inventory
            .iter()
            .find(|a| a.name == "Foo")
            .expect("Foo discovered")
            .clone();
        assert_eq!(app.bundle_id.as_deref(), Some("com.acme.foo"));

        let plan = plat.scan(&app, &inventory).unwrap();
        let paths: Vec<PathBuf> = plan.items.iter().map(|l| l.path.clone()).collect();
        let cases = [
            (bundle.clone(), true, "bundle"),
            (home.join("Library/Caches/com.acme.foo"), true, "caches"),
            (
                home.join("Library/Preferences/com.acme.foo.plist"),
                true,
                "prefs",
            ),
            (home.join("Library/Logs/com.acme.foo"), false, "absent logs"),
        ];
        for (path, expected, label) in cases {
            assert_eq!(paths.contains(&path), expected, "{label}");
        }
        assert!(plan.total_bytes > 0, "total size computed");
    }

    #[test]
    fn scan_includes_helper_excludes_sibling_and_foobar() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let apps = tmp.path().join("Applications");
        write(&apps.join("Foo.app/Contents/Info.plist"), INFO_PLIST_FOO);
        let bar = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.acme.foo.bar</string>
<key>CFBundleName</key><string>Bar</string>
</dict></plist>"#;
        write(&apps.join("Bar.app/Contents/Info.plist"), bar);
        write(&home.join("Library/Caches/com.acme.foo.helper/x"), "x");
        write(&home.join("Library/Caches/com.acme.foo.bar/y"), "y");
        write(&home.join("Library/Caches/com.acme.foobar/z"), "z");

        let plat = Platform::with_roots(home.clone(), vec![apps.clone()]);
        let inventory = plat.installed_apps().unwrap();
        let foo = inventory
            .iter()
            .find(|a| a.name == "Foo")
            .expect("Foo discovered")
            .clone();
        let plan = plat.scan(&foo, &inventory).unwrap();
        let ends = |suffix: &str| {
            plan.items
                .iter()
                .any(|l| l.path.to_string_lossy().ends_with(suffix))
        };

        assert!(ends("Caches/com.acme.foo.helper"), "helper kept");
        assert!(!ends("Caches/com.acme.foo.bar"), "sibling excluded");
        assert!(!ends("Caches/com.acme.foobar"), "foobar excluded");
        let helper = plan
            .items
            .iter()
            .find(|l| l.path.to_string_lossy().ends_with("foo.helper"))
            .expect("helper present");
        assert_eq!(helper.match_kind, MatchKind::Fuzzy, "non-exact is fuzzy");
    }

    #[test]
    fn is_protected_blocks_system_and_managed_apps() {
        let plat = Platform::with_roots(PathBuf::from("/Users/x"), vec![]);
        let cases = [
            (
                "/System/Applications/Mail.app",
                Some("com.apple.mail"),
                true,
            ),
            (
                "/Applications/Microsoft Defender.app",
                Some("com.microsoft.wdav.tray"),
                true,
            ),
            (
                "/Applications/CompanyPortal.app",
                Some("com.microsoft.intune.companyportal"),
                true,
            ),
            ("/Applications/Foo.app", Some("com.acme.foo"), false),
        ];
        for (path, bid, expected) in cases {
            let app = InstalledApp {
                name: "x".into(),
                bundle_id: bid.map(Into::into),
                path: PathBuf::from(path),
            };
            assert_eq!(plat.is_protected(&app), expected, "path: {path}");
        }
    }

    #[test]
    fn remove_paths_delete_removes_sources() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        let plat = Platform::with_roots(tmp.path().to_path_buf(), vec![]);
        let out = plat
            .remove_items(&[(tmp.path().join("a"), Disposal::Delete)])
            .unwrap();
        assert!(!tmp.path().join("a").exists(), "source deleted");
        assert_eq!(out.removed, vec![tmp.path().join("a")]);
        assert!(out.failed.is_empty());
    }
}
