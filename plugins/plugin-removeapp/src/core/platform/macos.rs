use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

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

    fn leftover_candidates(&self, app: &InstalledApp) -> Vec<(LeftoverKind, PathBuf)> {
        let lib = self.library();
        let mut out: Vec<(LeftoverKind, PathBuf)> = Vec::new();

        let mut keys: Vec<&str> = vec![app.name.as_str()];
        if let Some(bid) = &app.bundle_id {
            keys.push(bid.as_str());
        }
        for key in &keys {
            out.push((
                LeftoverKind::ApplicationSupport,
                lib.join("Application Support").join(key),
            ));
            out.push((LeftoverKind::Caches, lib.join("Caches").join(key)));
            out.push((LeftoverKind::Logs, lib.join("Logs").join(key)));
        }
        if let Some(bid) = &app.bundle_id {
            out.push((
                LeftoverKind::Preferences,
                lib.join("Preferences").join(format!("{bid}.plist")),
            ));
            out.push((LeftoverKind::Containers, lib.join("Containers").join(bid)));
            out.push((
                LeftoverKind::GroupContainers,
                lib.join("Group Containers").join(bid),
            ));
            out.push((
                LeftoverKind::SavedState,
                lib.join("Saved Application State")
                    .join(format!("{bid}.savedState")),
            ));
            out.push((
                LeftoverKind::HttpStorages,
                lib.join("HTTPStorages").join(bid),
            ));
            out.push((LeftoverKind::WebKit, lib.join("WebKit").join(bid)));
            out.push((
                LeftoverKind::LaunchAgent,
                lib.join("LaunchAgents").join(format!("{bid}.plist")),
            ));
        }
        out
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

    fn scan(&self, app: &InstalledApp) -> Result<RemovalPlan> {
        let mut items = vec![Leftover {
            path: app.path.clone(),
            kind: LeftoverKind::AppBundle,
            size_bytes: path_size(&app.path),
            match_kind: MatchKind::Exact,
        }];
        for (kind, path) in self.leftover_candidates(app) {
            if path.exists() {
                let size_bytes = path_size(&path);
                items.push(Leftover {
                    path,
                    kind,
                    size_bytes,
                    match_kind: MatchKind::Exact,
                });
            }
        }
        let total_bytes = items.iter().map(|l| l.size_bytes).sum();
        Ok(RemovalPlan {
            app: app.clone(),
            items,
            total_bytes,
        })
    }

    fn remove_paths(&self, paths: &[PathBuf], how: Disposal) -> Result<RemovalOutcome> {
        let mut outcome = RemovalOutcome::default();
        for path in paths {
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
        let app = plat
            .installed_apps()
            .unwrap()
            .into_iter()
            .find(|a| a.name == "Foo")
            .expect("Foo discovered");
        assert_eq!(app.bundle_id.as_deref(), Some("com.acme.foo"));

        let plan = plat.scan(&app).unwrap();
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
            .remove_paths(&[tmp.path().join("a")], Disposal::Delete)
            .unwrap();
        assert!(!tmp.path().join("a").exists(), "source deleted");
        assert_eq!(out.removed, vec![tmp.path().join("a")]);
        assert!(out.failed.is_empty());
    }
}
