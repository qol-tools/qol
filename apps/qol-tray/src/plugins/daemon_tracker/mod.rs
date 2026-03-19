use super::Plugin;
#[cfg(unix)]
use crate::file_io;
use crate::paths;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

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

#[cfg(unix)]
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

#[cfg(unix)]
pub(crate) fn dev_link_dirs() -> Vec<PathBuf> {
    #[cfg(feature = "dev")]
    return crate::paths::shared_config_dir()
        .map(|config_dir| {
            crate::dev::active_dev_links(&config_dir)
                .into_values()
                .collect()
        })
        .unwrap_or_default();

    #[cfg(not(feature = "dev"))]
    Vec::new()
}

#[cfg(unix)]
pub(crate) fn kill_from_pid_files() {
    let roots = ManagedRoots::load();
    for path in daemon_pid_files() {
        process_pid_file(&path, &roots);
    }
}

#[cfg(unix)]
fn process_pid_file(path: &Path, roots: &ManagedRoots) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for line in content.lines() {
        kill_pid_if_managed(line, roots);
    }
    let _ = std::fs::remove_file(path);
}

#[cfg(unix)]
fn kill_pid_if_managed(line: &str, roots: &ManagedRoots) {
    let Ok(pid) = line.trim().parse::<i32>() else {
        return;
    };
    if !crate::process_utils::is_pid_alive(pid) {
        crate::process_utils::reap_children_nonblocking();
        return;
    }
    let exe = platform::pid_exe_path(pid);
    let is_managed = exe.as_ref().is_some_and(|e| roots.contains(e));
    if !is_managed {
        if exe.is_some() {
            return;
        }
        log::info!(
            "Killing saved daemon pid {} (exe path unavailable — zombie or crashed)",
            pid
        );
    } else {
        log::info!(
            "Killing orphan daemon process: {} ({})",
            pid,
            exe.unwrap().display()
        );
    }
    crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(100));
    crate::process_utils::reap_children_nonblocking();
}

#[cfg(unix)]
fn resolved_children(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| file_io::canonical_or_original(&e.path()))
        .collect()
}

#[cfg(unix)]
pub(crate) struct ManagedRoots {
    installs_root: Option<PathBuf>,
    shared_plugins_root: Option<PathBuf>,
    dev_link_dirs: Vec<PathBuf>,
}

#[cfg(unix)]
impl ManagedRoots {
    pub(crate) fn load() -> Self {
        Self {
            installs_root: paths::installs_dir().ok(),
            shared_plugins_root: paths::plugins_dir().ok(),
            dev_link_dirs: dev_link_dirs(),
        }
    }

    /// Check if the given binary path belongs to a managed plugin.
    pub(crate) fn contains(&self, target: &Path) -> bool {
        let target = file_io::canonical_or_original(target);
        self.candidate_roots()
            .iter()
            .any(|root| target.starts_with(root))
    }

    fn candidate_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = self
            .dev_link_dirs
            .iter()
            .map(|d| file_io::canonical_or_original(d))
            .collect();

        if let Some(root) = &self.shared_plugins_root {
            roots.push(file_io::canonical_or_original(root));
            roots.extend(resolved_children(root));
        }

        if let Some(root) = &self.installs_root {
            roots.push(file_io::canonical_or_original(root));
            for child in resolved_children(root) {
                let plugins = child.join("plugins");
                roots.push(file_io::canonical_or_original(&plugins));
            }
        }

        roots
    }
}
