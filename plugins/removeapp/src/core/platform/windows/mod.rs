mod catalog;
mod launch;
mod registry;
mod store;

use std::collections::BTreeSet;
use std::fs;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_apps::shell_link::{LinkTarget, ShellLink};
use qol_windowing::platform::windows::top_level_windows;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

use crate::cli::PLUGIN_ID;
use crate::core::guards::{
    ManagedPackage, PackageIndex, PackageManager, PackageScope, PackageStatus,
};
use crate::core::{
    delete_path, dir_size, AppPlatform, Disposal, IdentitySnapshot, InstalledApp, Leftover,
    LeftoverKind, MatchKind, RemovalOutcome, RemovalPlan,
};

use self::catalog::{
    build_catalog, normalize, owned_keys, path_key, same_path, uninstall_launch, with_store,
    within, CatalogApp, Roots, Shortcut,
};

const REMOVE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const UNREGISTER_TIMEOUT: Duration = Duration::from_secs(15);
const UNREGISTER_POLL: Duration = Duration::from_millis(250);
const SELF_PREFIX: &str = "qol";
const STORE_DATA_DIR: &str = "Packages";
const PROTECTED_ENV_ROOTS: &[&str] = &[
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "USERPROFILE",
];

pub struct Platform {
    roots: Roots,
}

impl Default for Platform {
    fn default() -> Self {
        Self {
            roots: Roots {
                roaming: env_path("APPDATA"),
                local: env_path("LOCALAPPDATA"),
                program_data: env_path("ProgramData"),
                windows: env_path("SystemRoot").or_else(|| env_path("windir")),
                protected: PROTECTED_ENV_ROOTS
                    .iter()
                    .filter_map(|name| env_path(name))
                    .collect(),
            },
        }
    }
}

impl Platform {
    pub fn new() -> Self {
        Self::default()
    }

    fn catalog(&self) -> Vec<CatalogApp> {
        let catalog = build_catalog(
            registry::uninstall_entries(),
            start_menu_shortcuts(),
            &self.roots,
            Path::is_dir,
        );
        with_store(catalog, &store::packages())
    }

    fn store_data(&self, package: &store::StorePackage) -> Option<Leftover> {
        let dir = self
            .roots
            .local
            .as_ref()?
            .join(STORE_DATA_DIR)
            .join(&package.family_name);
        dir.is_dir().then(|| Leftover {
            size_bytes: dir_size(&dir),
            path: dir,
            kind: LeftoverKind::Data,
            match_kind: MatchKind::Exact,
        })
    }

    fn leftovers(&self, target: &CatalogApp, catalog: &[CatalogApp]) -> Vec<Leftover> {
        let keys = owned_keys(target, catalog);
        let other_installs: Vec<&PathBuf> = catalog
            .iter()
            .filter(|other| !same_path(&other.app.path, &target.app.path))
            .filter_map(|other| other.install_dir.as_ref().map(|(dir, _)| dir))
            .collect();
        let mut found = Vec::new();
        for (kind, root) in self.roots.data_roots() {
            let Ok(entries) = fs::read_dir(&root) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = normalize(&entry.file_name().to_string_lossy());
                let path = entry.path();
                let candidates = if keys.contains(&name) {
                    vec![path]
                } else if target.publisher_keys.contains(&name) && path.is_dir() {
                    matching_children(&path, &keys)
                } else {
                    Vec::new()
                };
                for candidate in candidates {
                    if other_installs.iter().any(|dir| within(dir, &candidate)) {
                        continue;
                    }
                    found.push(Leftover {
                        size_bytes: dir_size(&candidate),
                        path: candidate,
                        kind,
                        match_kind: MatchKind::Exact,
                    });
                }
            }
        }
        found
    }

    fn contains_current_exe(path: &Path) -> bool {
        std::env::current_exe().is_ok_and(|exe| within(&exe, path))
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn find(catalog: &[CatalogApp], app: &InstalledApp) -> Option<CatalogApp> {
    catalog
        .iter()
        .find(|candidate| same_path(&candidate.app.path, &app.path))
        .cloned()
}

fn matching_children(dir: &Path, keys: &BTreeSet<String>) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| keys.contains(&normalize(&entry.file_name().to_string_lossy())))
        .map(|entry| entry.path())
        .collect()
}

fn start_menu_shortcuts() -> Vec<Shortcut> {
    let mut shortcuts = Vec::new();
    for root in qol_apps::start_menu::start_menu_roots() {
        for entry in qol_apps::start_menu::scan_start_menu_root(&root) {
            let target = ShellLink::read(&entry.path).map_or(LinkTarget::Unknown, |link| {
                link.target(&entry.path, |name| std::env::var(name).ok())
            });
            shortcuts.push(Shortcut {
                folder: top_folder(&root.path, &entry.path),
                name: entry.name,
                path: entry.path,
                target,
            });
        }
    }
    shortcuts
}

fn top_folder(root: &Path, path: &Path) -> Option<String> {
    let mut components = path.strip_prefix(root).ok()?.components();
    let first = components.next()?;
    components.next()?;
    match first {
        Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
        _ => None,
    }
}

fn primary_rank(kind: LeftoverKind) -> u8 {
    match kind {
        LeftoverKind::AppBundle => 0,
        LeftoverKind::DesktopEntry => 1,
        _ => 2,
    }
}

struct MatchedProcess {
    pid: u32,
    identity: String,
}

fn matching_processes(install_dir: &Path) -> Vec<MatchedProcess> {
    if !install_dir.is_dir() {
        return Vec::new();
    }
    let own_pid = std::process::id();
    qol_app_icon::processes()
        .into_iter()
        .filter_map(|process| {
            u32::try_from(process.pid)
                .ok()
                .map(|pid| (pid, process.pid))
        })
        .filter(|(pid, _)| *pid != 0 && *pid != own_pid)
        .filter(|(_, raw)| {
            qol_app_icon::process_executable(*raw).is_some_and(|exe| within(&exe, install_dir))
        })
        .filter_map(|(pid, _)| {
            qol_process::process_identity(pid)
                .ok()
                .map(|identity| MatchedProcess { pid, identity })
        })
        .collect()
}

pub(crate) fn metadata_identity(
    path: &Path,
    _meta: &std::fs::Metadata,
) -> (Option<u64>, Option<u64>) {
    file_identity(path).map_or((None, None), |(volume, index)| (Some(volume), Some(index)))
}

fn file_identity(path: &Path) -> Option<(u64, u64)> {
    let file = fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return None;
    }
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Some((u64::from(info.dwVolumeSerialNumber), index))
}

impl AppPlatform for Platform {
    fn installed_apps(&self) -> Result<Vec<InstalledApp>> {
        Ok(self
            .catalog()
            .into_iter()
            .map(|candidate| candidate.app)
            .collect())
    }

    fn scan(&self, app: &InstalledApp, _inventory: &[InstalledApp]) -> Result<RemovalPlan> {
        let catalog = self.catalog();
        let target = find(&catalog, app)
            .with_context(|| format!("{PLUGIN_ID}: {} is no longer installed", app.name))?;
        let mut items = Vec::new();
        if let Some(package) = target.store() {
            items.extend(self.store_data(package));
        }
        if let Some((dir, match_kind)) = &target.install_dir {
            items.push(Leftover {
                path: dir.clone(),
                kind: LeftoverKind::AppBundle,
                size_bytes: dir_size(dir),
                match_kind: *match_kind,
            });
        }
        for shortcut in &target.shortcuts {
            items.push(Leftover {
                path: shortcut.clone(),
                kind: LeftoverKind::DesktopEntry,
                size_bytes: dir_size(shortcut),
                match_kind: MatchKind::Exact,
            });
        }
        items.extend(self.leftovers(&target, &catalog));
        items.sort_by(|left, right| {
            primary_rank(left.kind)
                .cmp(&primary_rank(right.kind))
                .then_with(|| left.path.cmp(&right.path))
        });
        let mut seen = BTreeSet::new();
        items.retain(|item| seen.insert(path_key(&item.path)));
        let snapshots = items
            .iter()
            .map(|item| IdentitySnapshot::capture(&item.path))
            .collect();
        let total_bytes = items.iter().map(|item| item.size_bytes).sum();
        Ok(RemovalPlan {
            app: app.clone(),
            items,
            total_bytes,
            snapshots,
        })
    }

    fn remove_items(&self, items: &[(PathBuf, Disposal)]) -> Result<RemovalOutcome> {
        let mut outcome = RemovalOutcome::default();
        for (path, disposal) in items {
            let result = match disposal {
                Disposal::Trash => trash::delete(path).map_err(|error| error.to_string()),
                Disposal::Delete => delete_path(path),
            };
            match result {
                Ok(()) => outcome.removed.push(path.clone()),
                Err(error) => outcome.failed.push((path.clone(), error)),
            }
        }
        Ok(outcome)
    }

    fn is_protected(&self, app: &InstalledApp) -> bool {
        if let Some(package) = store::packages()
            .iter()
            .find(|package| same_path(&package.install_dir, &app.path))
        {
            return package.protected;
        }
        let under_windows = self
            .roots
            .windows
            .as_ref()
            .is_some_and(|windows| within(&app.path, windows));
        if app.name.to_lowercase().starts_with(SELF_PREFIX)
            || under_windows
            || Self::contains_current_exe(&app.path)
        {
            return true;
        }
        find(&self.catalog(), app)
            .and_then(|candidate| candidate.entry().map(|entry| entry.no_remove))
            .unwrap_or(false)
    }

    fn is_running(&self, app: &InstalledApp) -> bool {
        !matching_processes(&app.path).is_empty()
    }

    fn quit(&self, app: &InstalledApp) -> Result<()> {
        let processes = matching_processes(&app.path);
        let pids: BTreeSet<u32> = processes.iter().map(|process| process.pid).collect();
        let mut asked = BTreeSet::new();
        for window in top_level_windows() {
            let Some(pid) = window.pid() else {
                continue;
            };
            if pids.contains(&pid) && window.is_switchable() && window.request_close() {
                asked.insert(pid);
            }
        }
        for process in processes
            .iter()
            .filter(|process| !asked.contains(&process.pid))
        {
            if !qol_process::process_identity_matches(process.pid, &process.identity) {
                continue;
            }
            if let Err(error) = qol_process::kill_pid(process.pid) {
                if qol_process::is_pid_alive(process.pid) {
                    return Err(error)
                        .context(format!("{PLUGIN_ID}: could not stop pid {}", process.pid));
                }
            }
        }
        Ok(())
    }

    fn package_index(&self, inventory: &[InstalledApp]) -> PackageIndex {
        let catalog = self.catalog();
        let mut index = PackageIndex::absent();
        for app in inventory {
            let Some(candidate) = find(&catalog, app) else {
                continue;
            };
            if let Some(package) = candidate.store() {
                let status = ManagedPackage::parse(
                    PackageManager::MicrosoftStore,
                    &package.full_name,
                    PackageScope::User,
                )
                .map_or_else(
                    || {
                        PackageStatus::Unavailable(format!(
                            "invalid package name {:?}",
                            package.full_name
                        ))
                    },
                    PackageStatus::Managed,
                );
                index.insert(app.path.clone(), status);
                continue;
            }
            let Some(entry) = candidate.entry() else {
                continue;
            };
            let status = match ManagedPackage::parse(
                PackageManager::Windows,
                &entry.location.key,
                entry.location.scope(),
            ) {
                Some(_) if uninstall_launch(entry).is_none() => PackageStatus::Unavailable(
                    format!("{} has no usable uninstall command", entry.name),
                ),
                Some(package) => PackageStatus::Managed(package),
                None => PackageStatus::Unavailable(format!(
                    "invalid uninstall key {:?}",
                    entry.location.key
                )),
            };
            index.insert(app.path.clone(), status);
        }
        index
    }

    fn trashes_install_remnant(&self) -> bool {
        true
    }

    fn uninstall_package(&self, app: &InstalledApp, package: &ManagedPackage) -> Result<()> {
        if package.manager() == PackageManager::MicrosoftStore {
            return uninstall_store_package(&self.catalog(), app, package);
        }
        if package.manager() != PackageManager::Windows {
            bail!(
                "{PLUGIN_ID}: {} packages are not supported on Windows",
                package.manager().label()
            );
        }
        let entry = find(&self.catalog(), app)
            .and_then(|candidate| candidate.entry().cloned())
            .filter(|entry| {
                entry.location.key == package.id() && entry.location.scope() == package.scope()
            })
            .with_context(|| {
                format!(
                    "{PLUGIN_ID}: the Windows registration for {} changed",
                    app.name
                )
            })?;
        let launch = uninstall_launch(&entry).with_context(|| {
            format!("{PLUGIN_ID}: {} has no usable uninstall command", app.name)
        })?;
        log::debug!(
            "[{PLUGIN_ID}] package-remove manager=windows id={} scope={:?}",
            package.id(),
            package.scope()
        );
        launch::run_and_wait(
            &launch,
            entry.location.scope() == PackageScope::System,
            REMOVE_TIMEOUT,
        )?;
        let deadline = Instant::now() + UNREGISTER_TIMEOUT;
        while registry::is_registered(&entry.location) {
            if Instant::now() >= deadline {
                bail!(
                    "{PLUGIN_ID}: {} is still installed after its uninstaller finished",
                    app.name
                );
            }
            std::thread::sleep(UNREGISTER_POLL);
        }
        Ok(())
    }
}

fn uninstall_store_package(
    catalog: &[CatalogApp],
    app: &InstalledApp,
    package: &ManagedPackage,
) -> Result<()> {
    let stored = find(catalog, app)
        .and_then(|candidate| candidate.store().cloned())
        .filter(|stored| stored.full_name == package.id() && !stored.protected)
        .with_context(|| {
            format!(
                "{PLUGIN_ID}: the Microsoft Store registration for {} changed",
                app.name
            )
        })?;
    log::debug!(
        "[{PLUGIN_ID}] package-remove manager=store id={}",
        stored.full_name
    );
    store::remove(&stored.full_name)?;
    if store::is_installed(&stored.full_name) {
        bail!(
            "{PLUGIN_ID}: {} is still installed after Windows removed it",
            app.name
        );
    }
    Ok(())
}
