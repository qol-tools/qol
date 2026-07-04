mod platform;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::paths;

pub(super) trait RestartPort: Send + Sync {
    fn resolve_restart_binary(&self) -> Option<PathBuf>;
    fn binary_at(&self, dir: &Path) -> PathBuf {
        qol_dev_build::tray::debug_binary_path(dir, platform::binary_name())
    }
    fn exec_restart(&self, binary: &Path) -> Result<(), String>;
}

pub(super) struct PlatformRestartPort;

impl RestartPort for PlatformRestartPort {
    fn resolve_restart_binary(&self) -> Option<PathBuf> {
        let mut candidates = Vec::new();

        let debug_binary = qol_dev_build::tray::debug_binary_path(
            &paths::repo_root_from_manifest_dir(),
            platform::binary_name(),
        );
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

fn strip_deleted_suffix(path: &Path) -> Option<PathBuf> {
    let raw = path.to_string_lossy();
    let stripped = raw.strip_suffix(" (deleted)")?;
    if stripped.is_empty() {
        return None;
    }
    Some(PathBuf::from(stripped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct TestRestartPort;

    impl RestartPort for TestRestartPort {
        fn resolve_restart_binary(&self) -> Option<PathBuf> {
            None
        }

        fn exec_restart(&self, _binary: &Path) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn binary_at_uses_workspace_target_for_member_roots() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("mono");
        let tray_root = workspace.join("apps").join("qol-tray");
        std::fs::create_dir_all(&tray_root).unwrap();
        std::fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nmembers = [\"apps/qol-tray\"]\nresolver = \"2\"\n",
        )
        .unwrap();
        std::fs::write(
            tray_root.join("Cargo.toml"),
            "[package]\nname = \"qol-tray\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
        )
        .unwrap();

        assert_eq!(
            TestRestartPort.binary_at(&tray_root),
            workspace
                .join("target")
                .join("debug")
                .join(platform::binary_name())
        );
    }
}
