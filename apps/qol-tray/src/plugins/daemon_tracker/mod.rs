use super::Plugin;
use crate::paths;
use std::path::{Path, PathBuf};

pub mod platform;

pub(super) fn daemon_pids_path() -> Option<PathBuf> {
    paths::config_dir().ok().map(|p| p.join(".daemon-pids"))
}

pub fn kill_orphan_daemons() {
    platform::kill_orphan_daemons();
}

pub fn clean_stale_sockets(plugins: &[Plugin]) {
    platform::clean_stale_sockets(plugins);
}

pub fn save_daemon_pids(pids: &[u32]) {
    let Some(path) = daemon_pids_path() else {
        return;
    };
    let content = pids
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(&path, content);
}

/// Collect all `.daemon-pids` files from the current config dir and all install directories.
pub(crate) fn daemon_pid_files() -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Some(current) = daemon_pids_path() {
        files.push(current);
    }

    let Some(installs_dir) = paths::installs_dir().ok() else {
        return files;
    };
    let Ok(entries) = std::fs::read_dir(installs_dir) else {
        return files;
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path().join(".daemon-pids");
        if path.exists() {
            files.push(path);
        }
    }

    files
}

/// Collect dev-link directories from the shared config, if available.
pub(crate) fn dev_link_dirs() -> Vec<PathBuf> {
    #[cfg(feature = "dev")]
    {
        let config_dir = match crate::paths::shared_config_dir() {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };
        crate::dev::load_dev_links(&config_dir)
            .into_values()
            .collect()
    }
    #[cfg(not(feature = "dev"))]
    {
        Vec::new()
    }
}

/// Kill orphan daemons found in PID files, verifying each is a managed plugin binary
/// via the platform-specific `pid_exe_path`.
pub(crate) fn kill_from_pid_files() {
    let roots = ManagedRoots::load();
    for path in daemon_pid_files() {
        process_pid_file(&path, &roots);
    }
}

fn process_pid_file(path: &Path, roots: &ManagedRoots) {
    let Ok(content) = std::fs::read_to_string(path) else { return; };
    for line in content.lines() {
        kill_pid_if_managed(line, roots);
    }
    let _ = std::fs::remove_file(path);
}

fn kill_pid_if_managed(line: &str, roots: &ManagedRoots) {
    let Ok(pid) = line.trim().parse::<i32>() else { return; };
    let Some(exe) = platform::pid_exe_path(pid) else { return; };
    if !roots.contains(&exe) { return; }
    if crate::process_utils::is_pid_alive(pid) {
        log::info!("Killing orphan daemon process: {} ({})", pid, exe.display());
        crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(100));
    }
}

fn resolve_path(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn resolved_children(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| resolve_path(&e.path()))
        .collect()
}

/// Pre-resolved set of directories whose binaries are managed by qol-tray.
pub(crate) struct ManagedRoots {
    installs_root: Option<PathBuf>,
    shared_plugins_root: Option<PathBuf>,
    dev_link_dirs: Vec<PathBuf>,
}

impl ManagedRoots {
    pub fn load() -> Self {
        Self {
            installs_root: paths::installs_dir().ok(),
            shared_plugins_root: paths::plugins_dir().ok(),
            dev_link_dirs: dev_link_dirs(),
        }
    }

    /// Check if the given binary path belongs to a managed plugin.
    pub fn contains(&self, target: &Path) -> bool {
        let target = resolve_path(target);
        self.candidate_roots()
            .iter()
            .any(|root| target.starts_with(root))
    }

    fn candidate_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = self.dev_link_dirs.iter().map(|d| resolve_path(d)).collect();

        if let Some(root) = &self.shared_plugins_root {
            roots.push(resolve_path(root));
            roots.extend(resolved_children(root));
        }

        if let Some(root) = &self.installs_root {
            roots.push(resolve_path(root));
            for child in resolved_children(root) {
                let plugins = child.join("plugins");
                roots.push(resolve_path(&plugins));
            }
        }

        roots
    }
}
