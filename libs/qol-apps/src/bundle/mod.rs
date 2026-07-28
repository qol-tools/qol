use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{AppEntry, AppRoot};

const EXCLUDED_LAUNCHERS: &[&str] = &["Spotlight", "Launchpad"];

mod platform;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstalledApp {
    pub name: String,
    pub bundle_id: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub enum Spotlight<'a> {
    Disabled,
    All,
    Roots(&'a [PathBuf]),
}

pub fn macos_cache_dir() -> Option<PathBuf> {
    platform::cache_dir()
}

pub fn macos_launcher_roots() -> Vec<AppRoot> {
    platform::launcher_roots()
}

pub fn scan_macos_launcher_root(root: &AppRoot) -> Vec<AppEntry> {
    let mut entries = Vec::new();
    collect_launcher_entries(&root.path, 0, root.max_depth, &mut entries);
    entries.retain(|entry| {
        !EXCLUDED_LAUNCHERS
            .iter()
            .any(|excluded| excluded.eq_ignore_ascii_case(&entry.name))
    });
    entries
}

pub fn macos_installed_apps(app_dirs: &[PathBuf], spotlight: Spotlight<'_>) -> Vec<InstalledApp> {
    let mut candidates = direct_child_paths(app_dirs);
    match spotlight {
        Spotlight::Disabled => {}
        Spotlight::All => candidates.extend(platform::spotlight_app_paths(&[])),
        Spotlight::Roots(roots) => {
            candidates.extend(platform::spotlight_app_paths(roots));
        }
    }
    macos_inventory_from_paths(candidates, app_dirs)
}

pub fn macos_inventory_from_paths(
    candidates: impl IntoIterator<Item = PathBuf>,
    preferred_roots: &[PathBuf],
) -> Vec<InstalledApp> {
    let mut best: HashMap<PathBuf, (usize, InstalledApp)> = HashMap::new();
    for path in candidates {
        let Some(app) = read_macos_app_bundle(path.clone()) else {
            continue;
        };
        let identity = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        let rank = location_rank(&path, preferred_roots);
        if best.get(&identity).is_none_or(|(seen, _)| rank < *seen) {
            best.insert(identity, (rank, app));
        }
    }
    let mut apps = best.into_values().map(|(_, app)| app).collect::<Vec<_>>();
    apps.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.path.cmp(&right.path))
    });
    apps
}

pub fn read_macos_app_bundle(path: PathBuf) -> Option<InstalledApp> {
    if !is_macos_app_bundle(&path) {
        return None;
    }
    let (bundle_id, bundle_name) = platform::bundle_info(&path);
    let name = bundle_name.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Unknown")
            .to_string()
    });
    Some(InstalledApp {
        name,
        bundle_id,
        path,
    })
}

pub fn is_macos_app_bundle(path: &Path) -> bool {
    if path.extension().and_then(|extension| extension.to_str()) != Some("app") {
        return false;
    }
    if path.ancestors().skip(1).any(|ancestor| {
        ancestor
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("app")
            || ancestor.file_name().is_some_and(|name| name == "Contents")
    }) {
        return false;
    }
    path.join("Contents/Info.plist").is_file()
}

fn collect_launcher_entries(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    entries: &mut Vec<AppEntry>,
) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if let Some(app) = launcher_entry(&path) {
            entries.push(app);
            continue;
        }
        if !path.is_dir() || depth >= max_depth {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        collect_launcher_entries(&path, depth + 1, max_depth, entries);
    }
}

fn launcher_entry(path: &Path) -> Option<AppEntry> {
    if !is_launcher_path(path) {
        return None;
    }
    Some(AppEntry {
        name: path.file_stem()?.to_str()?.to_string(),
        exec: platform::launcher_exec(path),
        path: path.to_path_buf(),
    })
}

fn is_launcher_path(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("app") || extension.eq_ignore_ascii_case("prefPane")
        })
}

fn direct_child_paths(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter_map(|root| fs::read_dir(root).ok())
        .flat_map(|entries| entries.flatten().map(|entry| entry.path()))
        .collect()
}

fn location_rank(path: &Path, roots: &[PathBuf]) -> usize {
    roots
        .iter()
        .position(|root| path.parent() == Some(root.as_path()))
        .unwrap_or(roots.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.acme.foo</string>
<key>CFBundleName</key><string>Foo</string>
</dict></plist>"#;

    fn write_bundle(path: &Path, plist: &str) {
        fs::create_dir_all(path.join("Contents")).unwrap();
        fs::write(path.join("Contents/Info.plist"), plist).unwrap();
    }

    fn named_plist(name: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>\
             <key>CFBundleName</key><string>{name}</string></dict></plist>"
        )
    }

    #[test]
    fn top_level_bundle_validation_rejects_helpers_and_incomplete_paths() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("Real.app");
        let helper = real.join("Contents/Helpers/Helper.app");
        write_bundle(&real, INFO_PLIST);
        write_bundle(&helper, INFO_PLIST);
        fs::create_dir_all(temp.path().join("Ghost.app")).unwrap();

        let cases = [
            (real, true),
            (helper, false),
            (temp.path().join("Ghost.app"), false),
            (temp.path().join("folder"), false),
        ];
        for (path, expected) in cases {
            assert_eq!(is_macos_app_bundle(&path), expected, "{}", path.display());
        }
    }

    #[test]
    fn inventory_dedupes_paths_without_collapsing_equal_names() {
        let temp = tempfile::tempdir().unwrap();
        let primary = temp.path().join("Applications");
        let secondary = temp.path().join("home/Applications");
        let duplicate = primary.join("Dupe.app");
        let other = secondary.join("Dupe.app");
        write_bundle(&duplicate, &named_plist("Dupe"));
        write_bundle(&other, &named_plist("Dupe"));

        let apps = macos_inventory_from_paths(
            [duplicate.clone(), duplicate.clone(), other.clone()],
            &[primary, secondary],
        );

        assert_eq!(apps.iter().filter(|app| app.name == "Dupe").count(), 2);
        assert_eq!(apps.iter().filter(|app| app.path == duplicate).count(), 1);
        assert!(apps.iter().any(|app| app.path == other));
    }

    #[test]
    fn launcher_scan_finds_apps_and_preference_panes_but_excludes_system_launchers() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("Nested/ChatGPT.app")).unwrap();
        fs::create_dir_all(temp.path().join("Color.prefPane")).unwrap();
        fs::create_dir_all(temp.path().join("Spotlight.app")).unwrap();
        let root = AppRoot {
            path: temp.path().to_path_buf(),
            max_depth: 2,
        };

        let entries = scan_macos_launcher_root(&root);

        assert!(entries.iter().any(|entry| entry.name == "ChatGPT"));
        assert!(entries.iter().any(|entry| entry.name == "Color"));
        assert!(entries.iter().all(|entry| entry.name != "Spotlight"));
    }

    #[test]
    fn direct_inventory_scans_each_configured_root() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("Applications");
        let second = temp.path().join("System/Applications");
        write_bundle(&first.join("Foo.app"), INFO_PLIST);
        write_bundle(&second.join("Bar.app"), &named_plist("Bar"));
        let roots = vec![first, second];

        let apps = macos_installed_apps(&roots, Spotlight::Disabled);

        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].name, "Bar");
        assert_eq!(apps[1].name, "Foo");
    }
}
