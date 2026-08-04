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
    qol_dev_build::tray::resolve_tray_root(repo_root, Path::new(env!("CARGO_MANIFEST_DIR")))
}
