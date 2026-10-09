use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use qol_apps::shell_link::LinkTarget;

use super::store::StorePackage;
use crate::core::guards::PackageScope;
use crate::core::{InstalledApp, LeftoverKind, MatchKind};

const UNINSTALL_SUBKEY: &str = r"Microsoft\Windows\CurrentVersion\Uninstall";
const SKIPPED_RELEASE_TYPES: &[&str] = &["hotfix", "security update", "service pack", "update"];
const MSI_PROGRAM: &str = "msiexec";
const MSI_ARGUMENTS: &str = "/qb- /norestart";
const PUBLISHER_SUFFIXES: &[&str] = &[
    "co",
    "corp",
    "corporation",
    "gmbh",
    "inc",
    "limited",
    "llc",
    "ltd",
    "sa",
    "srl",
];
const GENERIC_KEYS: &[&str] = &[
    "app",
    "application",
    "applications",
    "bin",
    "cache",
    "common files",
    "comms",
    "connecteddevicesplatform",
    "crashdumps",
    "current",
    "d3dscache",
    "google",
    "intel",
    "microsoft",
    "mozilla",
    "nvidia",
    "package cache",
    "packages",
    "programs",
    "publishers",
    "temp",
    "tools",
    "virtualstore",
    "windows",
];
const SYSTEM_SHORTCUT_FOLDERS: &[&str] = &[
    "accessibility",
    "accessories",
    "administrative tools",
    "maintenance",
    "startup",
    "system tools",
    "windows accessories",
    "windows administrative tools",
    "windows ease of access",
    "windows powershell",
    "windows system",
    "windows tools",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Hive {
    LocalMachine,
    CurrentUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum View {
    Native,
    Wow32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct KeyLocation {
    pub(super) hive: Hive,
    pub(super) view: View,
    pub(super) key: String,
}

impl KeyLocation {
    pub(super) fn scope(&self) -> PackageScope {
        match self.hive {
            Hive::LocalMachine => PackageScope::System,
            Hive::CurrentUser => PackageScope::User,
        }
    }

    pub(super) fn parent_path(hive: Hive, view: View) -> String {
        match (hive, view) {
            (Hive::LocalMachine, View::Wow32) => {
                format!(r"SOFTWARE\WOW6432Node\{UNINSTALL_SUBKEY}")
            }
            (Hive::LocalMachine, View::Native) => format!(r"SOFTWARE\{UNINSTALL_SUBKEY}"),
            (Hive::CurrentUser, _) => format!(r"Software\{UNINSTALL_SUBKEY}"),
        }
    }

    pub(super) fn registry_path(&self) -> PathBuf {
        let hive = match self.hive {
            Hive::LocalMachine => "HKEY_LOCAL_MACHINE",
            Hive::CurrentUser => "HKEY_CURRENT_USER",
        };
        PathBuf::from(format!(
            r"{hive}\{}\{}",
            Self::parent_path(self.hive, self.view),
            self.key
        ))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct RawValues {
    pub(super) display_name: Option<String>,
    pub(super) publisher: Option<String>,
    pub(super) uninstall_string: Option<String>,
    pub(super) quiet_uninstall_string: Option<String>,
    pub(super) install_location: Option<String>,
    pub(super) display_icon: Option<String>,
    pub(super) parent_key_name: Option<String>,
    pub(super) release_type: Option<String>,
    pub(super) system_component: bool,
    pub(super) windows_installer: bool,
    pub(super) no_remove: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UninstallEntry {
    pub(super) location: KeyLocation,
    pub(super) name: String,
    pub(super) publisher: Option<String>,
    pub(super) uninstall: String,
    pub(super) quiet: Option<String>,
    pub(super) install_location: Option<PathBuf>,
    pub(super) display_icon: Option<PathBuf>,
    pub(super) windows_installer: bool,
    pub(super) no_remove: bool,
}

pub(super) fn entry_from(location: KeyLocation, values: RawValues) -> Option<UninstallEntry> {
    let name = non_empty(values.display_name)?;
    let uninstall = non_empty(values.uninstall_string)?;
    let skipped_release = values.release_type.as_deref().is_some_and(|release| {
        SKIPPED_RELEASE_TYPES.contains(&release.trim().to_ascii_lowercase().as_str())
    });
    if values.system_component || non_empty(values.parent_key_name).is_some() || skipped_release {
        return None;
    }
    Some(UninstallEntry {
        location,
        name,
        publisher: non_empty(values.publisher),
        uninstall,
        quiet: non_empty(values.quiet_uninstall_string),
        install_location: non_empty(values.install_location).map(|raw| clean_path(&raw)),
        display_icon: non_empty(values.display_icon).map(|raw| icon_path(&raw)),
        windows_installer: values.windows_installer,
        no_remove: values.no_remove,
    })
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn clean_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim().trim_matches('"').trim_end_matches(['\\', '/']);
    PathBuf::from(trimmed)
}

fn icon_path(raw: &str) -> PathBuf {
    let raw = raw.trim();
    let without_index = match raw.rsplit_once(',') {
        Some((path, index)) if index.trim().trim_start_matches('-').parse::<i32>().is_ok() => path,
        _ => raw,
    };
    clean_path(without_index)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Launch {
    pub(super) program: String,
    pub(super) arguments: String,
}

pub(super) fn uninstall_launch(entry: &UninstallEntry) -> Option<Launch> {
    let command = split_command(entry.quiet.as_deref().unwrap_or(&entry.uninstall))?;
    let msi = entry.windows_installer || program_stem(&command.program) == MSI_PROGRAM;
    if msi {
        return product_code(&entry.location.key).map(|code| Launch {
            program: format!("{MSI_PROGRAM}.exe"),
            arguments: format!("/x {code} {MSI_ARGUMENTS}"),
        });
    }
    Some(command)
}

pub(super) fn split_command(raw: &str) -> Option<Launch> {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('"') {
        let (program, arguments) = rest.split_once('"')?;
        return (!program.trim().is_empty()).then(|| Launch {
            program: program.trim().to_string(),
            arguments: arguments.trim().to_string(),
        });
    }
    let lower = raw.to_ascii_lowercase();
    let exe_end = lower
        .match_indices(".exe")
        .map(|(index, _)| index + 4)
        .find(|end| lower[*end..].chars().next().is_none_or(char::is_whitespace));
    let split = exe_end.or_else(|| raw.find(char::is_whitespace));
    let (program, arguments) = match split {
        Some(end) => raw.split_at(end),
        None => (raw, ""),
    };
    (!program.is_empty()).then(|| Launch {
        program: program.to_string(),
        arguments: arguments.trim().to_string(),
    })
}

fn program_stem(program: &str) -> String {
    let name = program.rsplit(['\\', '/']).next().unwrap_or(program);
    let name = name.to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

fn product_code(key: &str) -> Option<&str> {
    let inner = key.strip_prefix('{')?.strip_suffix('}')?;
    let groups: Vec<&str> = inner.split('-').collect();
    let lengths = [8, 4, 4, 4, 12];
    let valid = groups.len() == lengths.len()
        && groups.iter().zip(lengths).all(|(group, length)| {
            group.len() == length && group.chars().all(|c| c.is_ascii_hexdigit())
        });
    valid.then_some(key)
}

#[derive(Debug, Clone)]
pub(super) struct Roots {
    pub(super) roaming: Option<PathBuf>,
    pub(super) local: Option<PathBuf>,
    pub(super) program_data: Option<PathBuf>,
    pub(super) windows: Option<PathBuf>,
    pub(super) protected: Vec<PathBuf>,
}

impl Roots {
    pub(super) fn data_roots(&self) -> Vec<(LeftoverKind, PathBuf)> {
        let mut roots = Vec::new();
        if let Some(roaming) = &self.roaming {
            roots.push((LeftoverKind::Config, roaming.clone()));
        }
        if let Some(local) = &self.local {
            roots.push((LeftoverKind::Data, local.clone()));
            roots.push((LeftoverKind::Data, local.join("Programs")));
        }
        if let Some(program_data) = &self.program_data {
            roots.push((LeftoverKind::Data, program_data.clone()));
        }
        roots
    }

    fn all_protected(&self) -> Vec<&PathBuf> {
        self.data_roots_paths()
            .chain(self.windows.iter())
            .chain(self.protected.iter())
            .collect()
    }

    fn data_roots_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.roaming
            .iter()
            .chain(self.local.iter())
            .chain(self.program_data.iter())
    }

    pub(super) fn is_safe_dir(&self, path: &Path) -> bool {
        let deep_enough = path
            .components()
            .filter(|component| matches!(component, Component::Normal(_)))
            .count()
            >= 2;
        let under_windows = self
            .windows
            .as_ref()
            .is_some_and(|windows| within(path, windows));
        let programs = self.local.as_ref().map(|local| local.join("Programs"));
        let is_root = self
            .all_protected()
            .into_iter()
            .chain(programs.iter())
            .any(|root| same_path(path, root));
        deep_enough && !under_windows && !is_root
    }

    fn is_system_target(&self, target: &LinkTarget) -> bool {
        match target {
            LinkTarget::ShellFolder => true,
            LinkTarget::Path(path) => self
                .windows
                .as_ref()
                .is_some_and(|windows| within(path, windows)),
            LinkTarget::Advertised | LinkTarget::Unknown => false,
        }
    }
}

pub(super) fn same_path(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
}

pub(super) fn within(child: &Path, parent: &Path) -> bool {
    let parent = path_key(parent);
    let child = path_key(child);
    !parent.is_empty() && (child == parent || child.starts_with(&format!("{parent}\\")))
}

pub(super) fn path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Source {
    Registry(UninstallEntry),
    Shortcut,
    Store(StorePackage),
}

#[derive(Debug, Clone)]
pub(super) struct CatalogApp {
    pub(super) app: InstalledApp,
    pub(super) source: Source,
    pub(super) install_dir: Option<(PathBuf, MatchKind)>,
    pub(super) shortcuts: Vec<PathBuf>,
    pub(super) keys: BTreeSet<String>,
    pub(super) publisher_keys: BTreeSet<String>,
}

impl CatalogApp {
    pub(super) fn entry(&self) -> Option<&UninstallEntry> {
        match &self.source {
            Source::Registry(entry) => Some(entry),
            Source::Shortcut | Source::Store(_) => None,
        }
    }

    pub(super) fn store(&self) -> Option<&StorePackage> {
        match &self.source {
            Source::Store(package) => Some(package),
            Source::Registry(_) | Source::Shortcut => None,
        }
    }
}

pub(super) struct Shortcut {
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) folder: Option<String>,
    pub(super) target: LinkTarget,
}

fn system_folders(shortcuts: &[Shortcut], roots: &Roots) -> BTreeSet<String> {
    let mut verdicts: BTreeMap<String, (bool, bool)> = BTreeMap::new();
    for shortcut in shortcuts {
        let Some(folder) = shortcut.folder.as_deref() else {
            continue;
        };
        let (system, app) = verdicts.entry(normalize(folder)).or_default();
        if roots.is_system_target(&shortcut.target) {
            *system = true;
        } else if shortcut.target != LinkTarget::Unknown {
            *app = true;
        }
    }
    verdicts
        .into_iter()
        .filter(|(_, (system, app))| *system && !*app)
        .map(|(folder, _)| folder)
        .chain(
            SYSTEM_SHORTCUT_FOLDERS
                .iter()
                .map(|folder| folder.to_string()),
        )
        .collect()
}

pub(super) fn build_catalog(
    entries: Vec<UninstallEntry>,
    shortcuts: Vec<Shortcut>,
    roots: &Roots,
    is_dir: impl Fn(&Path) -> bool,
) -> Vec<CatalogApp> {
    let mut apps: Vec<CatalogApp> = Vec::new();
    let mut claimed_paths: BTreeSet<String> = BTreeSet::new();
    let mut entries = entries;
    entries.sort_by(|left, right| left.location.cmp(&right.location));
    for entry in entries {
        let keys = entry_keys(&entry);
        let install_dir = install_dir(&entry, &keys, roots, &is_dir);
        let path = install_dir
            .as_ref()
            .map(|(dir, _)| dir.clone())
            .filter(|dir| !claimed_paths.contains(&path_key(dir)))
            .unwrap_or_else(|| entry.location.registry_path());
        claimed_paths.insert(path_key(&path));
        apps.push(CatalogApp {
            app: InstalledApp {
                name: entry.name.clone(),
                bundle_id: Some(entry.location.key.clone()),
                path,
            },
            publisher_keys: publisher_keys(entry.publisher.as_deref()),
            install_dir,
            shortcuts: Vec::new(),
            keys,
            source: Source::Registry(entry),
        });
    }
    let system_folders = system_folders(&shortcuts, roots);
    for shortcut in shortcuts {
        let folder_key = shortcut.folder.as_deref().map(normalize);
        if roots.is_system_target(&shortcut.target)
            || folder_key
                .as_ref()
                .is_some_and(|folder| system_folders.contains(folder))
        {
            continue;
        }
        let shortcut_key = normalize(&shortcut.name);
        let target = match &shortcut.target {
            LinkTarget::Path(path) => Some(path.as_path()),
            _ => None,
        };
        let installed_in = |app: &CatalogApp| {
            target
                .zip(app.install_dir.as_ref())
                .is_some_and(|(target, (dir, _))| within(target, dir))
        };
        let named = |app: &CatalogApp| {
            app.entry().is_some()
                && (app.keys.contains(&shortcut_key)
                    || folder_key
                        .as_ref()
                        .is_some_and(|folder| app.keys.contains(folder)))
        };
        let owner = apps
            .iter()
            .position(installed_in)
            .or_else(|| apps.iter().position(named));
        match owner {
            Some(owner) => apps[owner].shortcuts.push(shortcut.path),
            None if !claimed_paths.contains(&path_key(&shortcut.path)) => {
                let keys = usable_keys([shortcut.name.as_str()]);
                let install_dir = target
                    .and_then(Path::parent)
                    .filter(|dir| owns_dir(dir, &keys, roots, &is_dir))
                    .filter(|dir| !claimed_paths.contains(&path_key(dir)))
                    .map(|dir| (dir.to_path_buf(), MatchKind::Fuzzy));
                let path = install_dir
                    .as_ref()
                    .map_or_else(|| shortcut.path.clone(), |(dir, _)| dir.clone());
                claimed_paths.insert(path_key(&shortcut.path));
                claimed_paths.insert(path_key(&path));
                apps.push(CatalogApp {
                    app: InstalledApp {
                        name: shortcut.name.clone(),
                        bundle_id: None,
                        path,
                    },
                    source: Source::Shortcut,
                    install_dir,
                    shortcuts: vec![shortcut.path],
                    keys,
                    publisher_keys: BTreeSet::new(),
                });
            }
            None => {}
        }
    }
    sort_by_name(&mut apps);
    apps
}

pub(super) fn with_store(mut apps: Vec<CatalogApp>, packages: &[StorePackage]) -> Vec<CatalogApp> {
    let mut claimed: BTreeSet<String> = apps.iter().map(|app| path_key(&app.app.path)).collect();
    for package in packages {
        if !claimed.insert(path_key(&package.install_dir)) {
            continue;
        }
        apps.push(CatalogApp {
            app: InstalledApp {
                name: package.name.clone(),
                bundle_id: Some(package.family_name.clone()),
                path: package.install_dir.clone(),
            },
            source: Source::Store(package.clone()),
            install_dir: None,
            shortcuts: Vec::new(),
            keys: BTreeSet::new(),
            publisher_keys: BTreeSet::new(),
        });
    }
    sort_by_name(&mut apps);
    apps
}

fn sort_by_name(apps: &mut [CatalogApp]) {
    apps.sort_by(|left, right| {
        left.app
            .name
            .to_lowercase()
            .cmp(&right.app.name.to_lowercase())
            .then_with(|| left.app.path.cmp(&right.app.path))
    });
}

fn install_dir(
    entry: &UninstallEntry,
    keys: &BTreeSet<String>,
    roots: &Roots,
    is_dir: &impl Fn(&Path) -> bool,
) -> Option<(PathBuf, MatchKind)> {
    if let Some(location) = &entry.install_location {
        if is_dir(location) && roots.is_safe_dir(location) {
            return Some((location.clone(), MatchKind::Exact));
        }
    }
    let uninstaller = split_command(&entry.uninstall).map(|launch| PathBuf::from(launch.program));
    [entry.display_icon.clone(), uninstaller]
        .into_iter()
        .flatten()
        .flat_map(|file| {
            file.ancestors()
                .skip(1)
                .take(2)
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        })
        .find(|dir| owns_dir(dir, keys, roots, is_dir))
        .map(|dir| (dir, MatchKind::Fuzzy))
}

fn owns_dir(
    dir: &Path,
    keys: &BTreeSet<String>,
    roots: &Roots,
    is_dir: &impl Fn(&Path) -> bool,
) -> bool {
    is_dir(dir)
        && roots.is_safe_dir(dir)
        && dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| keys.iter().any(|key| name_matches(name, key)))
}

fn name_matches(dir_name: &str, key: &str) -> bool {
    let dir = alnum(dir_name);
    dir.len() >= 3 && !is_generic(&normalize(dir_name)) && alnum(key).starts_with(&dir)
}

fn alnum(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

pub(super) fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_start_matches('.')
        .to_lowercase()
}

fn is_generic(key: &str) -> bool {
    GENERIC_KEYS.contains(&key)
}

fn usable_keys<'a>(candidates: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    candidates
        .into_iter()
        .map(normalize)
        .filter(|key| {
            key.chars().filter(char::is_ascii_alphanumeric).count() >= 3 && !is_generic(key)
        })
        .collect()
}

pub(super) fn entry_keys(entry: &UninstallEntry) -> BTreeSet<String> {
    let base = base_name(&entry.name);
    let without_publisher =
        publisher_keys(entry.publisher.as_deref())
            .iter()
            .find_map(|publisher| {
                base.to_lowercase()
                    .strip_prefix(&format!("{publisher} "))
                    .map(str::to_string)
            });
    let location_name = entry
        .install_location
        .as_ref()
        .and_then(|location| location.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_string);
    let key_name = product_code(&entry.location.key)
        .is_none()
        .then(|| entry.location.key.clone());
    let candidates = [
        Some(entry.name.clone()),
        Some(base),
        without_publisher,
        location_name,
        key_name,
    ];
    usable_keys(candidates.iter().flatten().map(String::as_str))
}

fn base_name(name: &str) -> String {
    let name = name.split(" (").next().unwrap_or(name);
    let mut words: Vec<&str> = name.split_whitespace().collect();
    while words.len() > 1
        && words.last().is_some_and(|word| {
            word.trim_start_matches(['v', 'V'])
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        })
    {
        words.pop();
    }
    words.join(" ")
}

pub(super) fn publisher_keys(publisher: Option<&str>) -> BTreeSet<String> {
    let Some(publisher) = publisher else {
        return BTreeSet::new();
    };
    let cleaned: String = publisher
        .chars()
        .map(|c| if c == ',' || c == '.' { ' ' } else { c })
        .collect();
    let mut words: Vec<&str> = cleaned.split_whitespace().collect();
    let full = normalize(&words.join(" "));
    while words.len() > 1
        && words
            .last()
            .is_some_and(|word| PUBLISHER_SUFFIXES.contains(&word.to_lowercase().as_str()))
    {
        words.pop();
    }
    let stripped = normalize(&words.join(" "));
    let first = words.first().map(|word| normalize(word));
    [Some(full), Some(stripped), first]
        .into_iter()
        .flatten()
        .filter(|key| key.chars().filter(char::is_ascii_alphanumeric).count() >= 3)
        .collect()
}

pub(super) fn owned_keys(app: &CatalogApp, catalog: &[CatalogApp]) -> BTreeSet<String> {
    let mut owners: BTreeMap<&str, usize> = BTreeMap::new();
    for other in catalog {
        for key in &other.keys {
            *owners.entry(key.as_str()).or_insert(0) += 1;
        }
    }
    app.keys
        .iter()
        .filter(|key| owners.get(key.as_str()).copied().unwrap_or(0) <= 1)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(hive: Hive, view: View, key: &str) -> KeyLocation {
        KeyLocation {
            hive,
            view,
            key: key.to_string(),
        }
    }

    fn values(name: &str, uninstall: &str) -> RawValues {
        RawValues {
            display_name: Some(name.to_string()),
            uninstall_string: Some(uninstall.to_string()),
            ..RawValues::default()
        }
    }

    fn entry(key: &str, name: &str, uninstall: &str) -> UninstallEntry {
        entry_from(
            location(Hive::LocalMachine, View::Native, key),
            values(name, uninstall),
        )
        .unwrap()
    }

    fn roots() -> Roots {
        Roots {
            roaming: Some(PathBuf::from(r"C:\Users\me\AppData\Roaming")),
            local: Some(PathBuf::from(r"C:\Users\me\AppData\Local")),
            program_data: Some(PathBuf::from(r"C:\ProgramData")),
            windows: Some(PathBuf::from(r"C:\Windows")),
            protected: vec![
                PathBuf::from(r"C:\Program Files"),
                PathBuf::from(r"C:\Program Files (x86)"),
                PathBuf::from(r"C:\Users\me"),
            ],
        }
    }

    #[test]
    fn entries_without_name_or_uninstaller_and_updates_are_skipped() {
        let cases = [
            ("plain", values("Foo", "foo.exe"), true),
            ("no name", values("", "foo.exe"), false),
            ("no uninstaller", values("Foo", " "), false),
            (
                "system component",
                RawValues {
                    system_component: true,
                    ..values("Foo", "foo.exe")
                },
                false,
            ),
            (
                "child update",
                RawValues {
                    parent_key_name: Some("Office".into()),
                    ..values("Foo", "foo.exe")
                },
                false,
            ),
            (
                "hotfix",
                RawValues {
                    release_type: Some("Security Update".into()),
                    ..values("Foo", "foo.exe")
                },
                false,
            ),
        ];
        for (label, raw, expected) in cases {
            let parsed = entry_from(location(Hive::CurrentUser, View::Native, "Foo"), raw);
            assert_eq!(parsed.is_some(), expected, "{label}");
        }
    }

    #[test]
    fn display_icon_and_install_location_are_cleaned() {
        let raw = RawValues {
            install_location: Some(r#""C:\Program Files\Foo\""#.into()),
            display_icon: Some(r"C:\Program Files\Foo\foo.exe,-101".into()),
            ..values("Foo", "foo.exe")
        };
        let parsed = entry_from(location(Hive::LocalMachine, View::Native, "Foo"), raw).unwrap();
        assert_eq!(
            parsed.install_location,
            Some(PathBuf::from(r"C:\Program Files\Foo"))
        );
        assert_eq!(
            parsed.display_icon,
            Some(PathBuf::from(r"C:\Program Files\Foo\foo.exe"))
        );
    }

    #[test]
    fn install_dir_looks_one_folder_above_versioned_binaries() {
        let onedrive = r"C:\Users\me\AppData\Local\Microsoft\OneDrive";
        let cases = [
            (
                r"C:\Users\me\AppData\Local\Microsoft\OneDrive\25.087.0506.0001\OneDriveSetup.exe",
                Some(onedrive),
            ),
            (
                r"C:\Users\me\AppData\Local\Microsoft\OneDrive\OneDrive.exe",
                Some(onedrive),
            ),
            (
                r"C:\Users\me\AppData\Local\Microsoft\Edge\1.2.3\setup.exe",
                None,
            ),
        ];
        for (uninstaller, expected) in cases {
            let raw = RawValues {
                publisher: Some("Microsoft Corporation".into()),
                ..values(
                    "Microsoft OneDrive",
                    &format!("\"{uninstaller}\" /uninstall"),
                )
            };
            let entry = entry_from(
                location(Hive::CurrentUser, View::Native, "OneDriveSetup.exe"),
                raw,
            )
            .unwrap();
            let keys = entry_keys(&entry);
            let found = install_dir(&entry, &keys, &roots(), &|path: &Path| {
                Path::new(uninstaller)
                    .ancestors()
                    .skip(1)
                    .any(|dir| same_path(path, dir))
            });
            assert_eq!(
                found,
                expected.map(|dir| (PathBuf::from(dir), MatchKind::Fuzzy)),
                "{uninstaller}"
            );
        }
    }

    #[test]
    fn split_command_handles_quoted_and_unquoted_programs() {
        let cases = [
            (
                r#""C:\Program Files\Foo\uninst.exe" /S"#,
                r"C:\Program Files\Foo\uninst.exe",
                "/S",
            ),
            (
                r"C:\Program Files\Foo Bar\unins000.exe /SILENT",
                r"C:\Program Files\Foo Bar\unins000.exe",
                "/SILENT",
            ),
            (
                r"C:\Users\me\AppData\Local\Discord\Update.exe --uninstall",
                r"C:\Users\me\AppData\Local\Discord\Update.exe",
                "--uninstall",
            ),
            (r"C:\Tools\remove.exe", r"C:\Tools\remove.exe", ""),
            (
                r"RunDll32 C:\PROGRA~1\Foo\setup.dll,Uninstall",
                "RunDll32",
                r"C:\PROGRA~1\Foo\setup.dll,Uninstall",
            ),
        ];
        for (raw, program, arguments) in cases {
            assert_eq!(
                split_command(raw),
                Some(Launch {
                    program: program.to_string(),
                    arguments: arguments.to_string(),
                }),
                "{raw}"
            );
        }
        assert_eq!(split_command(r#""unterminated"#), None);
    }

    #[test]
    fn uninstall_launch_prefers_quiet_and_rewrites_msi_installs_to_remove() {
        let guid = "{23170F69-40C1-2702-2301-000001000000}";
        let msi = entry(guid, "7-Zip", &format!("MsiExec.exe /I{guid}"));
        let quiet = UninstallEntry {
            quiet: Some(r#""C:\Foo\uninst.exe" /S"#.into()),
            ..entry("Foo", "Foo", r#""C:\Foo\uninst.exe""#)
        };
        let msi_without_code = entry("Foo", "Foo", "MsiExec.exe /I{nope}");
        let cases = [
            (
                msi,
                Some(Launch {
                    program: "msiexec.exe".into(),
                    arguments: format!("/x {guid} /qb- /norestart"),
                }),
            ),
            (
                quiet,
                Some(Launch {
                    program: r"C:\Foo\uninst.exe".into(),
                    arguments: "/S".into(),
                }),
            ),
            (msi_without_code, None),
        ];
        for (entry, expected) in cases {
            assert_eq!(uninstall_launch(&entry), expected, "{}", entry.name);
        }
    }

    #[test]
    fn registry_paths_name_the_hive_and_view() {
        let cases = [
            (
                location(Hive::LocalMachine, View::Native, "Foo"),
                r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Foo",
            ),
            (
                location(Hive::LocalMachine, View::Wow32, "Foo"),
                r"HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Foo",
            ),
            (
                location(Hive::CurrentUser, View::Native, "Foo"),
                r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall\Foo",
            ),
        ];
        for (location, expected) in cases {
            assert_eq!(location.registry_path(), PathBuf::from(expected));
        }
    }

    #[test]
    fn keys_cover_name_base_name_publisher_free_name_and_folder() {
        let entry = UninstallEntry {
            publisher: Some("Google LLC".into()),
            install_location: Some(PathBuf::from(r"C:\Program Files\Google\Chrome\Application")),
            ..entry("Google Chrome", "Google Chrome 128.0.6613.120", "setup.exe")
        };
        let keys = entry_keys(&entry);
        for expected in ["google chrome 128.0.6613.120", "google chrome", "chrome"] {
            assert!(keys.contains(expected), "{expected} in {keys:?}");
        }
        assert!(!keys.contains("application"));
        let publishers = publisher_keys(Some("Mozilla Corporation"));
        assert!(publishers.contains("mozilla"));
        assert!(publishers.contains("mozilla corporation"));
    }

    #[test]
    fn safe_dirs_exclude_roots_and_the_windows_tree() {
        let roots = roots();
        let cases = [
            (r"C:\Program Files\Foo", true),
            (r"C:\Users\me\AppData\Local\Programs\Foo", true),
            (r"C:\Program Files", false),
            (r"c:\program files\", false),
            (r"C:\Users\me\AppData\Local\Programs", false),
            (r"C:\Windows\System32\Foo", false),
            (r"C:\Users\me", false),
            (r"C:\Foo", false),
        ];
        for (path, expected) in cases {
            assert_eq!(roots.is_safe_dir(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn catalog_pairs_shortcuts_with_registry_apps_and_keeps_orphans() {
        let firefox = UninstallEntry {
            install_location: Some(PathBuf::from(r"C:\Program Files\Mozilla Firefox")),
            ..entry(
                "Mozilla Firefox 128.0 (x64 en-US)",
                "Mozilla Firefox (x64 en-US)",
                r#""C:\Program Files\Mozilla Firefox\uninstall\helper.exe""#,
            )
        };
        let zip = entry(
            "7-Zip",
            "7-Zip 23.01 (x64)",
            r#""C:\Program Files\7-Zip\Uninstall.exe""#,
        );
        let shortcut = |name: &str, folder: Option<&str>| Shortcut {
            name: name.to_string(),
            path: PathBuf::from(format!(r"C:\Menu\{name}.lnk")),
            folder: folder.map(str::to_string),
            target: LinkTarget::Unknown,
        };
        let shortcuts = vec![
            shortcut("Firefox", None),
            shortcut("Mozilla Firefox", None),
            shortcut("7-Zip File Manager", Some("7-Zip")),
            shortcut("Portable Tool", None),
            shortcut("Notepad", Some("Accessories")),
        ];
        let dirs = [
            r"C:\Program Files\Mozilla Firefox",
            r"C:\Program Files\7-Zip",
        ];
        let catalog = build_catalog(vec![firefox, zip], shortcuts, &roots(), |path| {
            dirs.iter().any(|dir| same_path(path, Path::new(dir)))
        });

        let names: Vec<&str> = catalog.iter().map(|app| app.app.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "7-Zip 23.01 (x64)",
                "Firefox",
                "Mozilla Firefox (x64 en-US)",
                "Portable Tool"
            ]
        );
        let firefox = &catalog[2];
        assert_eq!(
            firefox.app.path,
            PathBuf::from(r"C:\Program Files\Mozilla Firefox")
        );
        assert_eq!(firefox.install_dir.as_ref().unwrap().1, MatchKind::Exact);
        assert_eq!(firefox.shortcuts.len(), 1);
        let zip = &catalog[0];
        assert_eq!(
            zip.install_dir,
            Some((PathBuf::from(r"C:\Program Files\7-Zip"), MatchKind::Fuzzy))
        );
        assert_eq!(
            zip.shortcuts,
            vec![PathBuf::from(r"C:\Menu\7-Zip File Manager.lnk")]
        );
        assert_eq!(catalog[3].entry(), None);
    }

    #[test]
    fn shortcut_targets_drop_system_entries_and_attach_install_dirs() {
        let shortcut = |name: &str, folder: Option<&str>, target: LinkTarget| Shortcut {
            name: name.to_string(),
            path: PathBuf::from(match folder {
                Some(folder) => format!(r"C:\Menu\{folder}\{name}.lnk"),
                None => format!(r"C:\Menu\{name}.lnk"),
            }),
            folder: folder.map(str::to_string),
            target,
        };
        let at = |path: &str| LinkTarget::Path(PathBuf::from(path));
        let firefox = (
            "Mozilla Firefox (x64 en-US)",
            r"C:\Program Files\Mozilla Firefox",
            0,
        );
        let cases = [
            (
                "shell folder",
                vec![shortcut("File Explorer", None, LinkTarget::ShellFolder)],
                vec![firefox],
            ),
            (
                "windows target",
                vec![shortcut(
                    "Administrative Tools",
                    None,
                    at(r"C:\WINDOWS\system32\control.exe"),
                )],
                vec![firefox],
            ),
            (
                "windows setup stub",
                vec![shortcut(
                    "Microsoft OneDrive",
                    None,
                    at(r"C:\Windows\System32\OneDriveSetup.exe"),
                )],
                vec![firefox],
            ),
            (
                "installed target",
                vec![shortcut(
                    "Microsoft OneDrive",
                    None,
                    at(r"C:\Program Files\Microsoft OneDrive\OneDrive.exe"),
                )],
                vec![
                    (
                        "Microsoft OneDrive",
                        r"C:\Program Files\Microsoft OneDrive",
                        1,
                    ),
                    firefox,
                ],
            ),
            (
                "target in an unrelated folder",
                vec![shortcut(
                    "Portable",
                    None,
                    at(r"C:\Users\me\Desktop\portable.exe"),
                )],
                vec![firefox, ("Portable", r"C:\Menu\Portable.lnk", 1)],
            ),
            (
                "target inside a registry install",
                vec![shortcut(
                    "Firefox Private Browsing",
                    None,
                    at(r"C:\Program Files\Mozilla Firefox\private_browsing.exe"),
                )],
                vec![(firefox.0, firefox.1, 1)],
            ),
            (
                "two shortcuts into one portable folder",
                vec![
                    shortcut("Tool", None, at(r"C:\Tools\Tool\tool.exe")),
                    shortcut("Tool Settings", None, at(r"C:\Tools\Tool\settings.exe")),
                ],
                vec![firefox, ("Tool", r"C:\Tools\Tool", 2)],
            ),
            (
                "folder of system shortcuts and unknowns",
                vec![
                    shortcut(
                        "Debugger",
                        Some("Debugging Tools"),
                        at(r"C:\Windows\System32\dbg.exe"),
                    ),
                    shortcut(
                        "Debugger Notes",
                        Some("Debugging Tools"),
                        LinkTarget::Unknown,
                    ),
                ],
                vec![firefox],
            ),
            (
                "folder with an installed app keeps it",
                vec![
                    shortcut(
                        "Office Help",
                        Some("Office Tools"),
                        at(r"C:\Windows\hh.exe"),
                    ),
                    shortcut("Office Word", Some("Office Tools"), LinkTarget::Advertised),
                ],
                vec![
                    firefox,
                    ("Office Word", r"C:\Menu\Office Tools\Office Word.lnk", 1),
                ],
            ),
        ];
        let entries = || {
            vec![UninstallEntry {
                install_location: Some(PathBuf::from(r"C:\Program Files\Mozilla Firefox")),
                ..entry(
                    "Mozilla Firefox 128.0 (x64 en-US)",
                    "Mozilla Firefox (x64 en-US)",
                    r#""C:\Program Files\Mozilla Firefox\uninstall\helper.exe""#,
                )
            }]
        };
        let dirs = [
            r"C:\Program Files\Mozilla Firefox",
            r"C:\Program Files\Microsoft OneDrive",
            r"C:\Tools\Tool",
            r"C:\Users\me\Desktop",
        ];
        for (label, shortcuts, expected) in cases {
            let catalog = build_catalog(entries(), shortcuts, &roots(), |path| {
                dirs.iter().any(|dir| same_path(path, Path::new(dir)))
            });
            let actual: Vec<(&str, PathBuf, usize)> = catalog
                .iter()
                .map(|app| {
                    (
                        app.app.name.as_str(),
                        app.app.path.clone(),
                        app.shortcuts.len(),
                    )
                })
                .collect();
            let expected: Vec<(&str, PathBuf, usize)> = expected
                .into_iter()
                .map(|(name, path, count)| (name, PathBuf::from(path), count))
                .collect();
            assert_eq!(actual, expected, "{label}");
        }
    }

    #[test]
    fn apps_sharing_an_install_dir_fall_back_to_their_registry_path() {
        let one = UninstallEntry {
            install_location: Some(PathBuf::from(r"C:\Program Files\Suite")),
            ..entry("A", "Suite", "a.exe")
        };
        let two = UninstallEntry {
            install_location: Some(PathBuf::from(r"C:\Program Files\Suite")),
            ..entry("B", "Suite Helper", "b.exe")
        };
        let catalog = build_catalog(vec![one, two], Vec::new(), &roots(), |_| true);
        let paths: BTreeSet<PathBuf> = catalog.iter().map(|app| app.app.path.clone()).collect();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(Path::new(r"C:\Program Files\Suite")));
    }

    #[test]
    fn shared_keys_are_not_owned_by_either_app() {
        let catalog = build_catalog(
            vec![entry("A", "Widget", "a.exe"), entry("B", "Widget", "b.exe")],
            Vec::new(),
            &roots(),
            |_| false,
        );
        assert!(owned_keys(&catalog[0], &catalog).is_empty());
    }

    #[test]
    fn store_packages_join_the_catalog_unless_their_folder_is_already_listed() {
        let package = |name: &str, dir: &str| StorePackage {
            full_name: format!("{name}_1.0.0.0_x64__abc"),
            family_name: format!("{name}_abc"),
            name: name.to_string(),
            install_dir: PathBuf::from(dir),
            protected: false,
        };
        let listed = UninstallEntry {
            install_location: Some(PathBuf::from(r"C:\Program Files\Widget")),
            ..entry("W", "Widget", "w.exe")
        };
        let catalog = with_store(
            build_catalog(vec![listed], Vec::new(), &roots(), |_| true),
            &[
                package(
                    "Calculator",
                    r"C:\Program Files\WindowsApps\Calc_1.0.0.0_x64__abc",
                ),
                package("Widget Store", r"C:\Program Files\Widget"),
            ],
        );
        let names: Vec<&str> = catalog.iter().map(|app| app.app.name.as_str()).collect();
        assert_eq!(names, ["Calculator", "Widget"]);
        let calculator = &catalog[0];
        assert_eq!(calculator.app.bundle_id.as_deref(), Some("Calculator_abc"));
        assert!(calculator.store().is_some());
        assert!(calculator.entry().is_none());
        assert!(calculator.install_dir.is_none());
    }
}
