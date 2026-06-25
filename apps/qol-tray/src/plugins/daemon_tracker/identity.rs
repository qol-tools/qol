#[cfg(unix)]
use crate::file_io;
#[cfg(unix)]
use crate::paths;
use std::path::{Path, PathBuf};

pub fn running_exe_path(pid: i32) -> Option<PathBuf> {
    #[cfg(unix)]
    return super::platform::pid_exe_path(pid);

    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

pub(super) fn is_host_binary(executable: &Path) -> bool {
    let Some(name) = executable.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name = name.strip_suffix(" (deleted)").unwrap_or(name);
    let name = name.strip_suffix(".exe").unwrap_or(name);
    let tray = crate::installer::binary_filename();
    let tray = tray.strip_suffix(".exe").unwrap_or(&tray);
    name == tray || name == "qol" || name == "qol-tray-doctor"
}

#[cfg(unix)]
fn dev_link_dirs() -> Vec<PathBuf> {
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

#[cfg(not(unix))]
pub(crate) struct ManagedRoots;

#[cfg(not(unix))]
impl ManagedRoots {
    pub(crate) fn load() -> Self {
        Self
    }
}
