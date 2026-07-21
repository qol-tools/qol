use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::Result;

use crate::core::classify::{normalize_entry, owner_of};
use crate::core::guards::{parse_cask_map, sanitize_stderr, CaskIndex, CaskToken};
use crate::core::{
    AppPlatform, Disposal, IdentitySnapshot, InstalledApp, Leftover, LeftoverKind, MatchKind,
    RemovalOutcome, RemovalPlan,
};

pub struct Platform {
    home: PathBuf,
    app_dirs: Vec<PathBuf>,
    spotlight: bool,
}

impl Default for Platform {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let app_dirs = vec![PathBuf::from("/Applications"), home.join("Applications")];
        Self {
            home,
            app_dirs,
            spotlight: true,
        }
    }
}

impl Platform {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_roots(home: PathBuf, app_dirs: Vec<PathBuf>) -> Self {
        Self {
            home,
            app_dirs,
            spotlight: false,
        }
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

const BREW_TIMEOUT: Duration = Duration::from_secs(5);
const STDERR_CAP: usize = 4096;

pub(crate) fn metadata_identity(meta: &std::fs::Metadata) -> (Option<u64>, Option<u64>) {
    use std::os::unix::fs::MetadataExt;
    (Some(meta.dev()), Some(meta.ino()))
}

fn resolve_brew(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.exists()).cloned()
}

fn brew_path() -> Option<PathBuf> {
    resolve_brew(&[
        PathBuf::from("/opt/homebrew/bin/brew"),
        PathBuf::from("/usr/local/bin/brew"),
    ])
}

fn run_brew(brew: &Path, args: &[&str]) -> Result<std::process::Output> {
    use std::io::Read;
    use wait_timeout::ChildExt;
    let mut child = Command::new(brew)
        .args(args)
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let mut out_pipe = child.stdout.take().expect("piped stdout");
    let mut err_pipe = child.stderr.take().expect("piped stderr");
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });
    let status = match child.wait_timeout(BREW_TIMEOUT)? {
        Some(status) => status,
        None => {
            let _ = child.kill();
            anyhow::bail!("brew timed out")
        }
    };
    Ok(std::process::Output {
        status,
        stdout: out_reader.join().unwrap_or_default(),
        stderr: err_reader.join().unwrap_or_default(),
    })
}

fn running_bundle_ids() -> Vec<String> {
    use objc2_app_kit::NSWorkspace;
    objc2::rc::autoreleasepool(|_| {
        let ws = NSWorkspace::sharedWorkspace();
        ws.runningApplications()
            .iter()
            .filter_map(|app| app.bundleIdentifier().map(|b| b.to_string()))
            .collect()
    })
}

fn terminate_bundle_id(bid: &str) -> bool {
    use objc2_app_kit::NSWorkspace;
    objc2::rc::autoreleasepool(|_| {
        let ws = NSWorkspace::sharedWorkspace();
        for app in ws.runningApplications().iter() {
            if app.bundleIdentifier().map(|b| b.to_string()).as_deref() == Some(bid) {
                return app.terminate();
            }
        }
        false
    })
}

impl AppPlatform for Platform {
    fn installed_apps(&self) -> Result<Vec<InstalledApp>> {
        let spotlight = if self.spotlight {
            qol_apps::Spotlight::All
        } else {
            qol_apps::Spotlight::Disabled
        };
        let mut apps = qol_apps::macos_installed_apps(&self.app_dirs, spotlight);
        apps.retain(|app| !app.path.starts_with("/System"));
        Ok(apps)
    }

    fn scan(&self, app: &InstalledApp, inventory: &[InstalledApp]) -> Result<RemovalPlan> {
        if fs::symlink_metadata(&app.path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            anyhow::bail!("removeapp: {} is a symlink; refusing", app.name);
        }

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

        let snapshots = items
            .iter()
            .map(|l| IdentitySnapshot::capture(&l.path))
            .collect();
        let total_bytes = items.iter().map(|l| l.size_bytes).sum();
        Ok(RemovalPlan {
            app: app.clone(),
            items,
            total_bytes,
            snapshots,
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

    fn is_running(&self, app: &InstalledApp) -> bool {
        let Some(bid) = &app.bundle_id else {
            return false;
        };
        running_bundle_ids().iter().any(|b| b == bid)
    }

    fn quit(&self, app: &InstalledApp) -> Result<()> {
        let Some(bid) = &app.bundle_id else {
            anyhow::bail!("removeapp: {} has no bundle id", app.name)
        };
        if terminate_bundle_id(bid) {
            Ok(())
        } else {
            anyhow::bail!("removeapp: could not quit {}", app.name)
        }
    }

    fn cask_index(&self) -> CaskIndex {
        let Some(brew) = brew_path() else {
            return CaskIndex::absent();
        };
        let output = match run_brew(&brew, &["info", "--cask", "--json=v2", "--installed"]) {
            Ok(o) if o.status.success() => o,
            Ok(o) => return CaskIndex::unavailable(sanitize_stderr(&o.stderr, STDERR_CAP)),
            Err(e) => return CaskIndex::unavailable(e.to_string()),
        };
        match parse_cask_map(&String::from_utf8_lossy(&output.stdout)) {
            Ok(m) => CaskIndex::from_map(m),
            Err(e) => CaskIndex::unavailable(format!("brew json: {e}")),
        }
    }

    fn brew_uninstall(&self, token: &CaskToken) -> Result<()> {
        let Some(brew) = brew_path() else {
            anyhow::bail!("removeapp: brew not found")
        };
        let out = run_brew(&brew, &["uninstall", "--cask", "--", token.as_str()])?;
        if out.status.success() {
            Ok(())
        } else {
            anyhow::bail!("{}", sanitize_stderr(&out.stderr, STDERR_CAP))
        }
    }

    fn is_protected(&self, app: &InstalledApp) -> bool {
        let path = match fs::canonicalize(&app.path) {
            Ok(canonical) => canonical,
            Err(_) if !app.path.exists() => app.path.clone(),
            Err(_) => return true,
        };
        if path.starts_with("/System") || path.starts_with("/Library/Apple") {
            return true;
        }
        if let Some(bid) = &app.bundle_id {
            if MANAGED_PREFIXES.iter().any(|p| bid.starts_with(p)) {
                return true;
            }
        }
        path.exists() && !is_writable(&path)
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
    fn resolve_brew_prefers_trusted_paths_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("opt");
        let b = tmp.path().join("usrlocal");
        fs::write(&a, "x").unwrap();
        fs::write(&b, "x").unwrap();
        assert_eq!(resolve_brew(&[a.clone(), b.clone()]), Some(a));
        let missing = tmp.path().join("nope");
        assert_eq!(resolve_brew(&[missing, b.clone()]), Some(b));
        assert_eq!(resolve_brew(&[tmp.path().join("x")]), None);
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

    fn plist_named(name: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>\
             <key>CFBundleName</key><string>{name}</string></dict></plist>"
        )
    }

    #[test]
    fn is_app_bundle_requires_real_top_level_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("Real.app/Contents/Info.plist"), INFO_PLIST_FOO);
        write(
            &root.join("Real.app/Contents/Helpers/Helper.app/Contents/Info.plist"),
            INFO_PLIST_FOO,
        );
        fs::create_dir_all(root.join("ghost.app")).unwrap();
        fs::create_dir_all(root.join("folder")).unwrap();

        let cases = [
            (root.join("Real.app"), true, "real bundle"),
            (
                root.join("Real.app/Contents/Helpers/Helper.app"),
                false,
                "nested helper",
            ),
            (root.join("ghost.app"), false, "no Info.plist"),
            (root.join("folder"), false, "not an app"),
        ];
        for (path, expected, label) in cases {
            assert_eq!(qol_apps::is_macos_app_bundle(&path), expected, "{label}");
        }
    }

    #[test]
    fn installed_apps_dedupes_same_path_but_preserves_same_name_apps() {
        let tmp = tempfile::tempdir().unwrap();
        let primary = tmp.path().join("Applications");
        let secondary = tmp.path().join("home/Applications");
        let dupe = primary.join("Dupe.app");
        write(&dupe.join("Contents/Info.plist"), &plist_named("Dupe"));
        write(
            &secondary.join("Other.app/Contents/Info.plist"),
            &plist_named("Dupe"),
        );
        write(
            &primary.join("Solo.app/Contents/Info.plist"),
            &plist_named("Solo"),
        );
        fs::create_dir_all(primary.join("ghost.app")).unwrap();

        let inv = qol_apps::macos_inventory_from_paths(
            vec![
                dupe.clone(),
                dupe.clone(),
                secondary.join("Other.app"),
                primary.join("Solo.app"),
                primary.join("ghost.app"),
            ],
            &[primary.clone(), secondary.clone()],
        );

        assert_eq!(
            inv.iter().filter(|a| a.name == "Dupe").count(),
            2,
            "same display name does not collapse distinct bundles"
        );
        assert_eq!(
            inv.iter().filter(|a| a.path == dupe).count(),
            1,
            "duplicate discovery of the same path collapses"
        );
        assert!(inv.iter().any(|a| a.name == "Solo"), "unique app kept");
        assert!(inv.iter().all(|a| a.name != "ghost"), "non-bundle dropped");
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
