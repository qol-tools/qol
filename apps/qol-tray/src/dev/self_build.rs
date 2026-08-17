use std::path::{Path, PathBuf};

use qol_dev_build::BuildResult;

pub fn build_qol_tray_self_with_progress<F>(repo_root: Option<&Path>, on_progress: F) -> BuildResult
where
    F: FnMut(u8, String),
{
    let repo_root = resolve_qol_tray_self_root(repo_root);
    qol_dev_build::tray::build_tray(
        &repo_root,
        &qol_dev_build::tray::DEV_TRAY_BINARIES,
        on_progress,
    )
}

pub fn resolve_qol_tray_self_root(repo_root: Option<&Path>) -> PathBuf {
    qol_dev_build::tray::resolve_tray_root(repo_root, &qol_tray_self_fallback())
}

#[cfg(feature = "dev")]
fn qol_tray_self_fallback() -> PathBuf {
    crate::paths::default_workspace_root()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf())
}

#[cfg(not(feature = "dev"))]
fn qol_tray_self_fallback() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_qol_tray_package(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"qol-tray\"\nversion = \"3.0.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
    }

    fn write_workspace(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"apps/qol-tray\"]\nresolver = \"2\"\n",
        )
        .unwrap();
    }

    #[cfg(feature = "dev")]
    fn record_default_workspace(base: &Path) {
        let config_dir = crate::paths::shared_config_dir().unwrap();
        std::fs::create_dir_all(config_dir.join("dev")).unwrap();
        std::fs::write(
            config_dir.join("dev").join("default-workspace.txt"),
            format!("{}\n", base.display()),
        )
        .unwrap();
    }

    #[cfg(feature = "dev")]
    #[test]
    fn recompile_root_stays_on_default_workspace_when_binary_was_built_from_a_worktree() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("base");
        let worktree = tmp
            .path()
            .join("worktrees")
            .join("diff-viewer")
            .join("qol-monorepo");
        write_workspace(&base);
        write_workspace(&worktree);
        write_qol_tray_package(&base.join("apps").join("qol-tray"));
        write_qol_tray_package(&worktree.join("apps").join("qol-tray"));
        let _guard = crate::paths::push_test_path_root(tmp.path());
        record_default_workspace(&base);

        assert_eq!(qol_tray_self_fallback(), base);

        assert_eq!(
            resolve_qol_tray_self_root(None),
            base.join("apps").join("qol-tray"),
            "a running binary built from the worktree must not drag the self-recompile root there"
        );
        assert_eq!(
            resolve_qol_tray_self_root(Some(&worktree)),
            worktree.join("apps").join("qol-tray"),
            "an explicit worktree selection still builds from that worktree"
        );
    }

    #[test]
    fn recompile_root_falls_back_to_manifest_dir_without_recorded_workspace() {
        let tmp = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(tmp.path());

        assert_eq!(
            qol_tray_self_fallback(),
            Path::new(env!("CARGO_MANIFEST_DIR"))
        );
    }
}
