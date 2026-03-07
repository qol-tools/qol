mod platform;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) trait RestartPort: Send + Sync {
    fn resolve_restart_binary(&self) -> Option<PathBuf>;
    fn binary_at(&self, dir: &Path) -> PathBuf {
        dir.join("target")
            .join("debug")
            .join(platform::binary_name())
    }
    fn exec_restart(&self, binary: &Path) -> Result<(), String>;
}

pub(super) struct PlatformRestartPort;

impl RestartPort for PlatformRestartPort {
    fn resolve_restart_binary(&self) -> Option<PathBuf> {
        let mut candidates = Vec::new();

        let debug_binary = main_repo_root()
            .join("target")
            .join("debug")
            .join(platform::binary_name());
        candidates.push(debug_binary);

        if let Ok(current) = std::env::current_exe() {
            candidates.push(current.clone());
            if let Some(stripped) = strip_deleted_suffix(&current) {
                candidates.push(stripped);
            }
        }

        candidates.into_iter().find(|candidate| candidate.is_file())
    }

    fn exec_restart(&self, binary: &Path) -> Result<(), String> {
        platform::exec_restart(binary)
    }
}

pub(super) fn default_restart_port() -> Arc<dyn RestartPort> {
    Arc::new(PlatformRestartPort)
}

fn main_repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut dir = manifest.as_path();
    loop {
        if dir.join(".worktrees").is_dir() {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return manifest,
        }
    }
}

fn strip_deleted_suffix(path: &Path) -> Option<PathBuf> {
    let raw = path.to_string_lossy();
    let stripped = raw.strip_suffix(" (deleted)")?;
    if stripped.is_empty() {
        return None;
    }
    Some(PathBuf::from(stripped))
}
