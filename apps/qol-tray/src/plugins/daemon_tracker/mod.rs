use super::Plugin;
#[cfg(unix)]
use crate::file_io;
use crate::paths;
use std::path::Path;
use std::path::PathBuf;

pub mod platform;

#[cfg(unix)]
fn legacy_daemon_pids_path() -> Option<PathBuf> {
    paths::config_dir().ok().map(|p| p.join(".daemon-pids"))
}

pub fn kill_orphan_daemons() {
    platform::kill_orphan_daemons();
}

pub fn clean_stale_sockets(plugins: &[Plugin]) {
    platform::clean_stale_sockets(plugins);
}

pub fn save_plugin_pid(pids_dir: &Path, plugin_id: &str, pid: u32) {
    let path = pids_dir.join(format!("{}.pid", plugin_id));
    let _ = std::fs::write(&path, pid.to_string());
}

pub fn remove_plugin_pid(pids_dir: &Path, plugin_id: &str) {
    let path = pids_dir.join(format!("{}.pid", plugin_id));
    let _ = std::fs::remove_file(&path);
}

pub fn list_tracked_pids(pids_dir: &Path) -> impl Iterator<Item = (String, u32)> {
    std::fs::read_dir(pids_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()?.to_str()? != "pid" {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            let pid: u32 = std::fs::read_to_string(&path).ok()?.trim().parse().ok()?;
            Some((id, pid))
        })
}

pub fn clear_all_pids(pids_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(pids_dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "pid") {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(unix)]
fn legacy_pid_files() -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Some(current) = legacy_daemon_pids_path() {
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
    let pids_dir = crate::paths::runtime_pids_dir();
    for (_, pid) in list_tracked_pids(&pids_dir) {
        kill_pid_if_managed(&(pid as i32).to_string(), &roots);
    }
    clear_all_pids(&pids_dir);

    for path in legacy_pid_files() {
        if path.exists() {
            process_pid_file(&path, &roots);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_and_load_pid_roundtrip() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "foo", 12345);

        let pid_file = tmp.path().join("foo.pid");
        assert!(pid_file.exists());

        let content = std::fs::read_to_string(&pid_file).unwrap();
        assert_eq!(content.trim(), "12345");
    }

    #[test]
    fn remove_plugin_pid_deletes_file() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "foo", 12345);
        remove_plugin_pid(tmp.path(), "foo");
        assert!(!tmp.path().join("foo.pid").exists());
    }

    #[test]
    fn remove_plugin_pid_noop_when_missing() {
        let tmp = TempDir::new().unwrap();
        remove_plugin_pid(tmp.path(), "nonexistent");
    }

    #[test]
    fn list_tracked_pids_returns_all_entries() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "a", 111);
        save_plugin_pid(tmp.path(), "b", 222);

        let mut pids: Vec<_> = list_tracked_pids(tmp.path()).collect();
        pids.sort_by_key(|(id, _)| id.clone());

        assert_eq!(pids.len(), 2);
        assert_eq!(pids[0], ("a".to_string(), 111));
        assert_eq!(pids[1], ("b".to_string(), 222));
    }

    #[test]
    fn list_tracked_pids_skips_corrupt_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("bad.pid"), "not-a-number").unwrap();
        save_plugin_pid(tmp.path(), "good", 42);

        let pids: Vec<_> = list_tracked_pids(tmp.path()).collect();
        assert_eq!(pids.len(), 1);
        assert_eq!(pids[0], ("good".to_string(), 42));
    }

    #[test]
    fn clear_all_pids_removes_all_pid_files() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "a", 1);
        save_plugin_pid(tmp.path(), "b", 2);
        clear_all_pids(tmp.path());
        assert!(list_tracked_pids(tmp.path()).next().is_none());
    }
}
