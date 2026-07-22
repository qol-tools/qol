use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use wait_timeout::ChildExt;

use crate::core::guards::{
    sanitize_stderr, ManagedPackage, PackageIndex, PackageManager, PackageScope, PackageStatus,
};
use crate::core::{
    AppPlatform, Disposal, IdentitySnapshot, InstalledApp, Leftover, LeftoverKind, MatchKind,
    RemovalOutcome, RemovalPlan,
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const REMOVE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const STDERR_CAP: usize = 4096;
const SELF_LAUNCHERS: &[&str] = &["qol-tray.desktop", "removeapp.desktop"];
const GENERIC_KEYS: &[&str] = &[
    "applications",
    "autostart",
    "backgrounds",
    "bash",
    "dbus-1",
    "desktop-directories",
    "env",
    "flatpak",
    "fonts",
    "icons",
    "java",
    "keyrings",
    "mime",
    "python",
    "python3",
    "sh",
    "sounds",
    "systemd",
    "themes",
    "thumbnails",
    "trash",
    "wine",
];

#[derive(Default)]
struct ToolPaths {
    dpkg_query: Option<PathBuf>,
    apt_get: Option<PathBuf>,
    pkexec: Option<PathBuf>,
    flatpak: Option<PathBuf>,
}

impl ToolPaths {
    fn discover() -> ToolPaths {
        ToolPaths {
            dpkg_query: resolve_tool(&["/usr/bin/dpkg-query", "/bin/dpkg-query"]),
            apt_get: resolve_tool(&["/usr/bin/apt-get", "/bin/apt-get"]),
            pkexec: resolve_tool(&["/usr/bin/pkexec", "/bin/pkexec"]),
            flatpak: resolve_tool(&["/usr/bin/flatpak", "/bin/flatpak"]),
        }
    }
}

pub struct Platform {
    home: Option<PathBuf>,
    app_roots: Option<Vec<qol_apps::AppRoot>>,
    tools: ToolPaths,
}

impl Default for Platform {
    fn default() -> Self {
        Self {
            home: std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .map(PathBuf::from)
                .filter(|home| home.is_absolute()),
            app_roots: None,
            tools: ToolPaths::discover(),
        }
    }
}

impl Platform {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn with_roots(home: PathBuf, app_roots: Vec<qol_apps::AppRoot>) -> Self {
        Self {
            home: Some(home),
            app_roots: Some(app_roots),
            tools: ToolPaths::default(),
        }
    }

    fn roots(&self) -> Vec<qol_apps::AppRoot> {
        self.app_roots
            .clone()
            .unwrap_or_else(qol_apps::desktop::linux_app_roots)
    }

    fn index_flatpaks(
        &self,
        inventory: &[InstalledApp],
        index: &mut PackageIndex,
    ) -> BTreeSet<PathBuf> {
        let mut candidates = Vec::new();
        for app in inventory {
            if let Some(id) = flatpak_id(app) {
                candidates.push((app, id));
            } else if is_flatpak_launcher_path(&app.path) {
                index.insert(
                    app.path.clone(),
                    PackageStatus::Unavailable(format!(
                        "cannot determine Flatpak app id for {}",
                        app.path.display()
                    )),
                );
            }
        }
        if candidates.is_empty() {
            return BTreeSet::new();
        }

        let Some(flatpak) = &self.tools.flatpak else {
            for (app, _) in candidates {
                index.insert(
                    app.path.clone(),
                    PackageStatus::Unavailable("flatpak command not found".into()),
                );
            }
            return BTreeSet::new();
        };
        let args = string_args(&["list", "--app", "--columns=application,installation"]);
        let installed = match run_command(flatpak, &args, QUERY_TIMEOUT) {
            Ok(output) if output.status.success() => parse_flatpak_list(&output.stdout),
            Ok(output) => {
                let reason = sanitize_stderr(&output.stderr, STDERR_CAP);
                for (app, _) in candidates {
                    index.insert(app.path.clone(), PackageStatus::Unavailable(reason.clone()));
                }
                return BTreeSet::new();
            }
            Err(error) => {
                for (app, _) in candidates {
                    index.insert(
                        app.path.clone(),
                        PackageStatus::Unavailable(error.to_string()),
                    );
                }
                return BTreeSet::new();
            }
        };

        let mut claimed = BTreeSet::new();
        for (app, id) in candidates {
            let Some(scopes) = installed.get(&id) else {
                index.insert(
                    app.path.clone(),
                    PackageStatus::Unavailable(format!(
                        "{id} is not present in Flatpak's installed app list"
                    )),
                );
                continue;
            };
            let scope = match select_flatpak_scope(&app.path, scopes) {
                Ok(scope) => scope,
                Err(reason) => {
                    index.insert(
                        app.path.clone(),
                        PackageStatus::Unavailable(reason.to_string()),
                    );
                    continue;
                }
            };
            let Some(package) = ManagedPackage::parse(PackageManager::Flatpak, &id, scope) else {
                index.insert(
                    app.path.clone(),
                    PackageStatus::Unavailable(format!("invalid Flatpak app id {id:?}")),
                );
                continue;
            };
            claimed.insert(app.path.clone());
            index.insert(app.path.clone(), PackageStatus::Managed(package));
        }
        claimed
    }

    fn index_apt(
        &self,
        inventory: &[InstalledApp],
        claimed: &BTreeSet<PathBuf>,
        index: &mut PackageIndex,
    ) {
        let candidates: Vec<&InstalledApp> = inventory
            .iter()
            .filter(|app| !claimed.contains(&app.path) && is_dpkg_launcher_path(&app.path))
            .collect();
        if candidates.is_empty() {
            return;
        }
        let Some(dpkg_query) = &self.tools.dpkg_query else {
            for app in candidates {
                index.insert(
                    app.path.clone(),
                    PackageStatus::Unavailable("dpkg-query command not found".into()),
                );
            }
            return;
        };

        let mut args = string_args(&["--search", "--"]);
        args.extend(candidates.iter().map(|app| app.path.as_os_str().to_owned()));
        let output = match run_command(dpkg_query, &args, QUERY_TIMEOUT) {
            Ok(output) => output,
            Err(error) => {
                for app in candidates {
                    index.insert(
                        app.path.clone(),
                        PackageStatus::Unavailable(error.to_string()),
                    );
                }
                return;
            }
        };
        let owners = parse_dpkg_search(&output.stdout);
        for app in candidates {
            let status = match owners.get(&app.path) {
                Some(packages) if packages.len() == 1 => {
                    ManagedPackage::parse(PackageManager::Apt, &packages[0], PackageScope::System)
                        .map(PackageStatus::Managed)
                        .unwrap_or_else(|| {
                            PackageStatus::Unavailable(format!(
                                "invalid dpkg package id {:?}",
                                packages[0]
                            ))
                        })
                }
                Some(packages) => PackageStatus::Unavailable(format!(
                    "{} packages own {}",
                    packages.len(),
                    app.path.display()
                )),
                None => PackageStatus::Unavailable(format!(
                    "no dpkg package owns {}",
                    app.path.display()
                )),
            };
            index.insert(app.path.clone(), status);
        }
    }

    fn uninstall_apt(&self, app: &InstalledApp, package: &ManagedPackage) -> Result<()> {
        let apt_get = self
            .tools
            .apt_get
            .as_ref()
            .context("removeapp: apt-get not found")?;
        let dpkg_query = self
            .tools
            .dpkg_query
            .as_ref()
            .context("removeapp: dpkg-query not found")?;
        let pkexec = self
            .tools
            .pkexec
            .as_ref()
            .context("removeapp: pkexec not found")?;

        ensure_dpkg_owns_launcher(dpkg_query, &app.path, package.id())?;
        ensure_apt_package_is_removable(dpkg_query, package.id())?;
        eprintln!(
            "[removeapp] package-preflight manager=apt id={}",
            package.id()
        );
        let simulation = run_command(
            apt_get,
            &string_args(&["--simulate", "purge", "--", package.id()]),
            QUERY_TIMEOUT,
        )?;
        if !simulation.status.success() {
            anyhow::bail!(
                "removeapp: apt preflight failed: {}",
                sanitize_stderr(&simulation.stderr, STDERR_CAP)
            );
        }
        ensure_apt_removes_only_target(&simulation.stdout, package.id())?;

        eprintln!(
            "[removeapp] package-remove manager=apt id={} scope=system",
            package.id()
        );
        let args = vec![
            apt_get.as_os_str().to_owned(),
            OsString::from("--yes"),
            OsString::from("purge"),
            OsString::from("--"),
            OsString::from(package.id()),
        ];
        let output = run_command(pkexec, &args, REMOVE_TIMEOUT)?;
        ensure_success("apt", output)
    }

    fn uninstall_flatpak(&self, app: &InstalledApp, package: &ManagedPackage) -> Result<()> {
        let flatpak = self
            .tools
            .flatpak
            .as_ref()
            .context("removeapp: flatpak not found")?;
        let scope = match package.scope() {
            PackageScope::User => "--user",
            PackageScope::System => "--system",
        };
        if flatpak_id(app).as_deref() != Some(package.id()) {
            anyhow::bail!(
                "removeapp: Flatpak ownership changed for {}",
                app.path.display()
            )
        }
        let info = run_command(
            flatpak,
            &string_args(&["info", scope, package.id()]),
            QUERY_TIMEOUT,
        )?;
        if !info.status.success() {
            anyhow::bail!(
                "removeapp: cannot confirm Flatpak ownership: {}",
                sanitize_stderr(&info.stderr, STDERR_CAP)
            )
        }
        eprintln!(
            "[removeapp] package-remove manager=flatpak id={} scope={scope}",
            package.id()
        );
        let output = run_command(
            flatpak,
            &string_args(&[
                "uninstall",
                scope,
                "--app",
                "--noninteractive",
                "--assumeyes",
                package.id(),
            ]),
            REMOVE_TIMEOUT,
        )?;
        ensure_success("flatpak", output)
    }
}

pub(crate) fn metadata_identity(meta: &std::fs::Metadata) -> (Option<u64>, Option<u64>) {
    use std::os::unix::fs::MetadataExt;
    (Some(meta.dev()), Some(meta.ino()))
}

impl AppPlatform for Platform {
    fn installed_apps(&self) -> Result<Vec<InstalledApp>> {
        let mut by_identity = BTreeMap::new();
        for root in self.roots() {
            for entry in qol_apps::desktop::scan_desktop_root(&root) {
                if is_qol_launcher(&entry.path) {
                    continue;
                }
                let identity = fs::canonicalize(&entry.path).unwrap_or_else(|_| entry.path.clone());
                by_identity.entry(identity).or_insert(InstalledApp {
                    name: entry.name,
                    bundle_id: None,
                    path: entry.path,
                });
            }
        }
        let mut apps: Vec<InstalledApp> = by_identity.into_values().collect();
        apps.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.path.cmp(&right.path))
        });
        Ok(apps)
    }

    fn scan(&self, app: &InstalledApp, inventory: &[InstalledApp]) -> Result<RemovalPlan> {
        let desktop = qol_apps::desktop::parse_desktop_entry_file(&app.path)
            .with_context(|| format!("removeapp: cannot read {}", app.path.display()))?;
        let mut items = vec![Leftover {
            path: app.path.clone(),
            kind: LeftoverKind::DesktopEntry,
            size_bytes: path_size(&app.path),
            match_kind: MatchKind::Exact,
        }];

        if let Some(executable) = self
            .home
            .as_deref()
            .and_then(|home| user_executable(&desktop, home))
        {
            let shared = inventory.iter().any(|other| {
                other.path != app.path
                    && qol_apps::desktop::parse_desktop_entry_file(&other.path)
                        .and_then(|entry| {
                            self.home
                                .as_deref()
                                .and_then(|home| user_executable(&entry, home))
                        })
                        .is_some_and(|path| path == executable)
            });
            if !shared {
                items.push(Leftover {
                    size_bytes: path_size(&executable),
                    path: executable,
                    kind: LeftoverKind::ApplicationBinary,
                    match_kind: MatchKind::Exact,
                });
            }
        }

        let owners = key_owner_counts(inventory);
        let keys: BTreeSet<String> = desktop_keys(&desktop)
            .into_iter()
            .filter(|key| owners.get(key).copied().unwrap_or(0) <= 1)
            .collect();
        if let Some(home) = &self.home {
            for (kind, root) in linux_data_roots(home) {
                let Ok(entries) = fs::read_dir(root) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let key = normalize_key(&entry.file_name().to_string_lossy());
                    if !keys.contains(&key) {
                        continue;
                    }
                    let path = entry.path();
                    items.push(Leftover {
                        size_bytes: path_size(&path),
                        path,
                        kind,
                        match_kind: MatchKind::Exact,
                    });
                }
            }
        }

        items.sort_by(|left, right| {
            primary_rank(left.kind)
                .cmp(&primary_rank(right.kind))
                .then_with(|| left.path.cmp(&right.path))
        });
        items.dedup_by(|left, right| left.path == right.path);
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
        let basename = app.path.file_name().and_then(|name| name.to_str());
        if basename.is_some_and(|name| SELF_LAUNCHERS.contains(&name)) || is_qol_launcher(&app.path)
        {
            return true;
        }
        if is_dpkg_launcher_path(&app.path) || is_flatpak_launcher_path(&app.path) {
            return false;
        }
        app.path.is_absolute()
            && self
                .home
                .as_ref()
                .is_none_or(|home| !app.path.starts_with(home))
    }

    fn is_running(&self, app: &InstalledApp) -> bool {
        !matching_processes(app).is_empty()
    }

    fn quit(&self, app: &InstalledApp) -> Result<()> {
        let processes = matching_processes(app);
        if processes.is_empty() {
            return Ok(());
        }
        for process in processes {
            if !qol_process::process_identity_matches(process.pid, &process.identity) {
                continue;
            }
            if let Err(error) = qol_process::signal_term_pid(process.pid) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(error)
                        .context(format!("removeapp: could not stop pid {}", process.pid));
                }
            }
        }
        Ok(())
    }

    fn package_index(&self, inventory: &[InstalledApp]) -> PackageIndex {
        let mut index = PackageIndex::absent();
        let claimed = self.index_flatpaks(inventory, &mut index);
        self.index_apt(inventory, &claimed, &mut index);
        index
    }

    fn uninstall_package(&self, app: &InstalledApp, package: &ManagedPackage) -> Result<()> {
        match package.manager() {
            PackageManager::Apt => self.uninstall_apt(app, package),
            PackageManager::Flatpak => self.uninstall_flatpak(app, package),
            PackageManager::Homebrew => {
                anyhow::bail!("removeapp: Homebrew packages are not supported on Linux")
            }
        }
    }
}

fn resolve_tool(candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

fn string_args(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn run_command(program: &Path, args: &[OsString], timeout: Duration) -> Result<Output> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    qol_process::isolate_owned_command(&mut command)
        .context("removeapp: failed to isolate package command")?;
    let mut child = command
        .spawn()
        .with_context(|| format!("removeapp: failed to run {}", program.display()))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let status = match child.wait_timeout(timeout)? {
        Some(status) => status,
        None => {
            let _ = qol_process::kill_group(child.id());
            let _ = child.wait();
            anyhow::bail!("removeapp: {} timed out", program.display())
        }
    };
    Ok(Output {
        status,
        stdout: stdout_reader.join().unwrap_or_default(),
        stderr: stderr_reader.join().unwrap_or_default(),
    })
}

fn ensure_success(manager: &str, output: Output) -> Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "removeapp: {manager} uninstall failed: {}",
            sanitize_stderr(&output.stderr, STDERR_CAP)
        )
    }
}

fn parse_flatpak_list(raw: &[u8]) -> BTreeMap<String, BTreeSet<PackageScope>> {
    let mut installed: BTreeMap<String, BTreeSet<PackageScope>> = BTreeMap::new();
    for line in String::from_utf8_lossy(raw).lines() {
        let mut fields = line.split('\t');
        let Some(id) = fields.next().map(str::trim) else {
            continue;
        };
        let scope = match fields.next().map(str::trim) {
            Some("user") => PackageScope::User,
            Some("system") => PackageScope::System,
            _ => continue,
        };
        let Some(package) = ManagedPackage::parse(PackageManager::Flatpak, id, scope) else {
            continue;
        };
        installed
            .entry(package.id().to_string())
            .or_default()
            .insert(scope);
    }
    installed
}

fn select_flatpak_scope(path: &Path, scopes: &BTreeSet<PackageScope>) -> Result<PackageScope> {
    let declared_scope = if path.starts_with("/var/lib/flatpak/exports") {
        Some(PackageScope::System)
    } else if path
        .to_string_lossy()
        .contains("/.local/share/flatpak/exports/")
    {
        Some(PackageScope::User)
    } else {
        None
    };
    if let Some(scope) = declared_scope {
        if scopes.contains(&scope) {
            return Ok(scope);
        }
        anyhow::bail!(
            "Flatpak launcher scope does not match the installed app at {}",
            path.display()
        )
    }
    if scopes.len() == 1 {
        return Ok(*scopes.iter().next().expect("one scope"));
    }
    anyhow::bail!(
        "Flatpak app is installed in multiple scopes; cannot classify {}",
        path.display()
    )
}

fn parse_dpkg_search(raw: &[u8]) -> BTreeMap<PathBuf, Vec<String>> {
    let mut owners: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for line in String::from_utf8_lossy(raw).lines() {
        let Some((packages, path)) = line.split_once(": ") else {
            continue;
        };
        let entry = owners.entry(PathBuf::from(path)).or_default();
        for package in packages.split(',').map(str::trim) {
            if ManagedPackage::parse(PackageManager::Apt, package, PackageScope::System).is_some() {
                entry.insert(package.to_string());
            }
        }
    }
    owners
        .into_iter()
        .map(|(path, packages)| (path, packages.into_iter().collect()))
        .collect()
}

fn ensure_dpkg_owns_launcher(dpkg_query: &Path, launcher: &Path, package: &str) -> Result<()> {
    let mut args = string_args(&["--search", "--"]);
    args.push(launcher.as_os_str().to_owned());
    let output = run_command(dpkg_query, &args, QUERY_TIMEOUT)?;
    let owners = parse_dpkg_search(&output.stdout);
    let matches = owners.get(launcher).map(Vec::as_slice).unwrap_or_default();
    if output.status.success() && matches == [package] {
        return Ok(());
    }
    anyhow::bail!(
        "removeapp: dpkg ownership changed for {}",
        launcher.display()
    )
}

fn ensure_apt_package_is_removable(dpkg_query: &Path, package: &str) -> Result<()> {
    let output = run_command(
        dpkg_query,
        &string_args(&[
            "--show",
            "--showformat=${db:Status-Abbrev}\\t${Essential}\\t${Priority}\\n",
            "--",
            package,
        ]),
        QUERY_TIMEOUT,
    )?;
    if !output.status.success() {
        anyhow::bail!(
            "removeapp: cannot inspect apt package {package}: {}",
            sanitize_stderr(&output.stderr, STDERR_CAP)
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut fields = text.trim().split('\t');
    let status = fields.next().unwrap_or_default();
    let essential = fields.next().unwrap_or_default();
    let priority = fields.next().unwrap_or_default();
    if !status.starts_with("ii") {
        anyhow::bail!("removeapp: apt package {package} is not installed")
    }
    if essential == "yes" || matches!(priority, "required" | "important") {
        anyhow::bail!("removeapp: refusing to remove protected apt package {package} ({priority})")
    }
    Ok(())
}

fn ensure_apt_removes_only_target(raw: &[u8], target: &str) -> Result<()> {
    let removals: BTreeSet<String> = String::from_utf8_lossy(raw)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            matches!(fields.next(), Some("Remv" | "Purg"))
                .then(|| fields.next().map(str::to_string))
                .flatten()
        })
        .collect();
    if removals.len() == 1
        && removals
            .iter()
            .next()
            .is_some_and(|package| package_ids_match(package, target))
    {
        return Ok(());
    }
    if removals.is_empty() {
        anyhow::bail!("removeapp: apt preflight did not plan removal of {target}")
    }
    anyhow::bail!(
        "removeapp: apt would also remove {}; use the system package manager to review that plan",
        removals.into_iter().collect::<Vec<_>>().join(", ")
    )
}

fn package_base(package: &str) -> &str {
    package.split(':').next().unwrap_or(package)
}

fn package_ids_match(actual: &str, expected: &str) -> bool {
    actual == expected
        || (!actual.contains(':') && actual == package_base(expected))
        || (!expected.contains(':') && package_base(actual) == expected)
}

fn is_dpkg_launcher_path(path: &Path) -> bool {
    path.starts_with("/usr/share/applications")
}

fn is_flatpak_launcher_path(path: &Path) -> bool {
    path.to_string_lossy().contains("/flatpak/exports/")
}

fn is_qol_launcher(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == "qol-tray.desktop"
                || name.starts_with("qol-shortcut-")
                || name.starts_with("qol-plugin-settings-")
                || name.starts_with("qol-command-")
        })
}

fn flatpak_id(app: &InstalledApp) -> Option<String> {
    let path_looks_flatpak = is_flatpak_launcher_path(&app.path);
    if path_looks_flatpak {
        let id = app.path.file_stem()?.to_str()?;
        return ManagedPackage::parse(PackageManager::Flatpak, id, PackageScope::System)
            .map(|package| package.id().to_string());
    }
    let entry = qol_apps::desktop::parse_desktop_entry_file(&app.path)?;
    let run_index = entry.exec.iter().position(|arg| {
        Path::new(arg)
            .file_name()
            .is_some_and(|name| name == "flatpak")
    })? + 1;
    let id = entry.exec[run_index..]
        .iter()
        .skip_while(|arg| arg.starts_with('-') || arg.as_str() == "run")
        .find(|arg| !arg.starts_with('-'))?;
    if !entry.exec.iter().any(|arg| arg == "run") {
        return None;
    }
    ManagedPackage::parse(PackageManager::Flatpak, id, PackageScope::System)
        .map(|package| package.id().to_string())
}

fn linux_data_roots(home: &Path) -> Vec<(LeftoverKind, PathBuf)> {
    vec![
        (LeftoverKind::Config, home.join(".config")),
        (LeftoverKind::Caches, home.join(".cache")),
        (LeftoverKind::Data, home.join(".local/share")),
        (LeftoverKind::Data, home.join(".local/state")),
        (LeftoverKind::Data, home.join(".var/app")),
    ]
}

fn key_owner_counts(inventory: &[InstalledApp]) -> BTreeMap<String, usize> {
    let mut owners = BTreeMap::new();
    for app in inventory {
        let Some(entry) = qol_apps::desktop::parse_desktop_entry_file(&app.path) else {
            continue;
        };
        for key in desktop_keys(&entry) {
            *owners.entry(key).or_insert(0) += 1;
        }
    }
    owners
}

fn desktop_keys(entry: &qol_apps::AppEntry) -> BTreeSet<String> {
    let mut candidates = Vec::new();
    if let Some(stem) = entry.path.file_stem().and_then(|stem| stem.to_str()) {
        candidates.push(stem.to_string());
    }
    candidates.push(entry.name.clone());
    if let Some(program) = executable_token(&entry.exec) {
        if let Some(name) = Path::new(program)
            .file_name()
            .and_then(|name| name.to_str())
        {
            candidates.push(name.to_string());
        }
    }
    if let Some(id) = flatpak_id(&InstalledApp {
        name: entry.name.clone(),
        bundle_id: None,
        path: entry.path.clone(),
    }) {
        candidates.push(id);
    }
    candidates
        .into_iter()
        .map(|candidate| normalize_key(&candidate))
        .filter(|key| key.len() >= 3 && !GENERIC_KEYS.contains(&key.as_str()))
        .collect()
}

fn normalize_key(value: &str) -> String {
    value.trim().trim_start_matches('.').to_ascii_lowercase()
}

fn user_executable(entry: &qol_apps::AppEntry, home: &Path) -> Option<PathBuf> {
    let token = executable_token(&entry.exec)?;
    let resolved = resolve_executable(token)?;
    let canonical = fs::canonicalize(&resolved).ok()?;
    (canonical.starts_with(home) && canonical.is_file() && !is_shared_process_launcher(&canonical))
        .then_some(canonical)
}

fn executable_token(exec: &[String]) -> Option<&str> {
    let mut iter = exec.iter().map(String::as_str);
    let first = iter.next()?;
    if Path::new(first)
        .file_name()
        .is_some_and(|name| name == "env")
    {
        return iter.find(|arg| !arg.starts_with('-') && !arg.contains('='));
    }
    Some(first)
}

fn resolve_executable(program: &str) -> Option<PathBuf> {
    let path = PathBuf::from(program);
    if path.is_absolute() {
        return path.is_file().then_some(path);
    }
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .filter(|segment| !segment.is_empty())
        .map(|segment| Path::new(segment).join(program))
        .find(|candidate| candidate.is_file())
}

enum ProcessTarget {
    Executable {
        canonical: PathBuf,
        argv_paths: Vec<Vec<u8>>,
    },
    CommandLineArg(Vec<u8>),
}

fn process_target(app: &InstalledApp) -> Option<ProcessTarget> {
    if let Some(id) = flatpak_id(app) {
        return Some(ProcessTarget::CommandLineArg(id.into_bytes()));
    }
    let entry = qol_apps::desktop::parse_desktop_entry_file(&app.path)?;
    let executable = resolve_executable(executable_token(&entry.exec)?)?;
    let canonical = fs::canonicalize(&executable).ok()?;
    if is_shared_process_launcher(&canonical) {
        return None;
    }
    let mut argv_paths = vec![executable.as_os_str().as_bytes().to_vec()];
    if canonical != executable {
        argv_paths.push(canonical.as_os_str().as_bytes().to_vec());
    }
    Some(ProcessTarget::Executable {
        canonical,
        argv_paths,
    })
}

fn is_shared_process_launcher(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "bash"
            | "dash"
            | "electron"
            | "env"
            | "fish"
            | "flatpak"
            | "gio"
            | "java"
            | "node"
            | "nodejs"
            | "perl"
            | "php"
            | "ruby"
            | "sh"
            | "snap"
            | "steam"
            | "wine"
            | "wine64"
            | "xdg-open"
            | "zsh"
    ) || name.strip_prefix("python").is_some_and(|suffix| {
        suffix.is_empty()
            || suffix
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
    })
}

fn command_line_has_arg(command_line: &[u8], expected: &[u8]) -> bool {
    command_line
        .split(|byte| *byte == 0)
        .any(|argument| argument == expected)
}

struct MatchedProcess {
    pid: u32,
    identity: String,
}

fn matching_processes(app: &InstalledApp) -> Vec<MatchedProcess> {
    let Some(target) = process_target(app) else {
        return Vec::new();
    };
    let Ok(processes) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let own_pid = std::process::id();
    processes
        .flatten()
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
        .filter(|pid| *pid != own_pid)
        .filter(|pid| match &target {
            ProcessTarget::Executable {
                canonical,
                argv_paths,
            } => {
                let executable_matches = fs::read_link(format!("/proc/{pid}/exe"))
                    .ok()
                    .and_then(|path| fs::canonicalize(path).ok())
                    .is_some_and(|path| path == *canonical);
                executable_matches
                    || fs::read(format!("/proc/{pid}/cmdline"))
                        .ok()
                        .is_some_and(|bytes| {
                            argv_paths
                                .iter()
                                .any(|expected| command_line_has_arg(&bytes, expected))
                        })
            }
            ProcessTarget::CommandLineArg(expected) => fs::read(format!("/proc/{pid}/cmdline"))
                .ok()
                .is_some_and(|bytes| command_line_has_arg(&bytes, expected)),
        })
        .filter_map(|pid| {
            qol_process::process_identity(pid)
                .ok()
                .map(|identity| MatchedProcess { pid, identity })
        })
        .collect()
}

fn primary_rank(kind: LeftoverKind) -> u8 {
    match kind {
        LeftoverKind::ApplicationBinary => 0,
        LeftoverKind::DesktopEntry => 1,
        _ => 2,
    }
}

fn path_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.file_type().is_symlink() {
        return 0;
    }
    if metadata.is_file() {
        return metadata.len();
    }
    fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| path_size(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

fn delete_path(path: &Path) -> std::result::Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn root(path: PathBuf) -> qol_apps::AppRoot {
        qol_apps::AppRoot { path, max_depth: 1 }
    }

    #[test]
    fn installed_apps_and_scan_find_user_binary_and_exact_data() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let applications = home.join(".local/share/applications");
        let binary = home.join("Applications/widget.AppImage");
        write(&binary, "binary");
        write(
            &applications.join("widget.desktop"),
            &format!("[Desktop Entry]\nName=Widget\nExec={}\n", binary.display()),
        );
        write(
            &applications.join("qol-shortcut-plugin-removeapp-open.desktop"),
            "[Desktop Entry]\nName=Remove App\nExec=qol-tray exec shortcut removeapp\n",
        );
        write(&home.join(".config/widget/settings"), "config");
        write(&home.join(".cache/widgetish/keep"), "near miss");

        let platform = Platform::with_roots(home.clone(), vec![root(applications)]);
        let inventory = platform.installed_apps().unwrap();
        assert_eq!(inventory.len(), 1);
        assert_eq!(inventory[0].name, "Widget");

        let plan = platform.scan(&inventory[0], &inventory).unwrap();
        assert!(plan
            .items
            .iter()
            .any(|item| { item.kind == LeftoverKind::ApplicationBinary && item.path == binary }));
        assert!(plan
            .items
            .iter()
            .any(|item| item.path == home.join(".config/widget")));
        assert!(!plan
            .items
            .iter()
            .any(|item| item.path == home.join(".cache/widgetish")));
        assert_eq!(plan.items.len(), plan.snapshots.len());
    }

    #[test]
    fn shared_executable_and_config_key_are_not_planned_twice() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let applications = home.join(".local/share/applications");
        let binary = home.join(".local/bin/shared");
        write(&binary, "binary");
        for name in ["One", "Two"] {
            write(
                &applications.join(format!("{name}.desktop")),
                &format!("[Desktop Entry]\nName={name}\nExec={}\n", binary.display()),
            );
        }
        write(&home.join(".config/shared/settings"), "config");

        let platform = Platform::with_roots(home.clone(), vec![root(applications)]);
        let inventory = platform.installed_apps().unwrap();
        let plan = platform.scan(&inventory[0], &inventory).unwrap();
        assert!(!plan.items.iter().any(|item| item.path == binary));
        assert!(!plan
            .items
            .iter()
            .any(|item| item.path == home.join(".config/shared")));
    }

    #[test]
    fn parses_flatpak_inventory_and_dpkg_ownership() {
        let flatpaks = parse_flatpak_list(
            b"org.example.One\tuser\norg.example.Two\tsystem\norg.example.Both\tuser\norg.example.Both\tsystem\norg.example.Bad\tcustom\n",
        );
        assert_eq!(
            flatpaks["org.example.One"],
            BTreeSet::from([PackageScope::User])
        );
        assert_eq!(
            flatpaks["org.example.Two"],
            BTreeSet::from([PackageScope::System])
        );
        assert_eq!(flatpaks["org.example.Both"].len(), 2);
        assert!(!flatpaks.contains_key("org.example.Bad"));

        let both = &flatpaks["org.example.Both"];
        assert_eq!(
            select_flatpak_scope(
                Path::new(
                    "/home/test/.local/share/flatpak/exports/share/applications/org.example.Both.desktop"
                ),
                both,
            )
            .unwrap(),
            PackageScope::User
        );
        assert_eq!(
            select_flatpak_scope(
                Path::new("/var/lib/flatpak/exports/share/applications/org.example.Both.desktop"),
                both,
            )
            .unwrap(),
            PackageScope::System
        );
        assert!(select_flatpak_scope(Path::new("/tmp/custom.desktop"), both).is_err());

        let owners = parse_dpkg_search(
            b"firefox: /usr/share/applications/firefox.desktop\nfoo, bar: /x.desktop\n",
        );
        assert_eq!(
            owners[Path::new("/usr/share/applications/firefox.desktop")],
            vec!["firefox"]
        );
        assert_eq!(owners[Path::new("/x.desktop")], vec!["bar", "foo"]);
    }

    #[test]
    fn apt_preflight_accepts_only_the_selected_package() {
        ensure_apt_removes_only_target(b"Purg firefox [1.0]\n", "firefox").unwrap();
        ensure_apt_removes_only_target(b"Purg firefox:amd64 [1.0]\n", "firefox").unwrap();
        ensure_apt_removes_only_target(b"Purg firefox [1.0]\n", "firefox:amd64").unwrap();
        assert!(
            ensure_apt_removes_only_target(b"Purg firefox:i386 [1.0]\n", "firefox:amd64").is_err()
        );
        let error = ensure_apt_removes_only_target(
            b"Remv mint-meta-cinnamon [1]\nRemv cinnamon [2]\n",
            "cinnamon",
        )
        .unwrap_err();
        assert!(error.to_string().contains("also remove"));
    }

    #[test]
    fn leftover_keys_exclude_shared_xdg_directories_and_generic_runtimes() {
        let entry = qol_apps::AppEntry {
            name: "Applications".into(),
            exec: vec!["python3".into()],
            path: PathBuf::from("/tmp/icons.desktop"),
        };
        let keys = desktop_keys(&entry);

        assert!(!keys.contains("applications"));
        assert!(!keys.contains("icons"));
        assert!(!keys.contains("python3"));
    }

    #[test]
    fn self_launcher_is_protected_but_dpkg_launchers_defer_to_package_guard() {
        let platform = Platform::with_roots(PathBuf::from("/home/test"), vec![]);
        let app = |path: &str| InstalledApp {
            name: "App".into(),
            bundle_id: None,
            path: PathBuf::from(path),
        };
        assert!(platform.is_protected(&app("/usr/share/applications/qol-tray.desktop")));
        assert!(!platform.is_protected(&app("/usr/share/applications/firefox.desktop")));
        assert!(platform.is_protected(&app("/opt/vendor/app.desktop")));
    }

    #[test]
    fn missing_home_protects_user_paths() {
        let platform = Platform {
            home: None,
            app_roots: Some(vec![]),
            tools: ToolPaths::default(),
        };
        let app = InstalledApp {
            name: "User App".into(),
            bundle_id: None,
            path: PathBuf::from("/home/test/.local/share/applications/user.desktop"),
        };

        assert!(platform.is_protected(&app));
    }

    #[test]
    fn shared_runtime_processes_are_never_matched_by_executable() {
        for path in [
            "/usr/bin/python3.12",
            "/usr/bin/sh",
            "/usr/bin/electron",
            "/usr/bin/steam",
        ] {
            assert!(is_shared_process_launcher(Path::new(path)), "{path}");
        }
        assert!(!is_shared_process_launcher(Path::new("/usr/bin/onboard")));
        assert!(command_line_has_arg(
            b"/usr/bin/python3\0/usr/bin/onboard\0",
            b"/usr/bin/onboard"
        ));
        assert!(!command_line_has_arg(
            b"/usr/bin/python3\0/usr/bin/other\0",
            b"/usr/bin/onboard"
        ));
    }
}
