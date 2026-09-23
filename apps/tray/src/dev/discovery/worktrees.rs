use std::path::{Path, PathBuf};

pub(super) fn active_worktree_plugin_dirs(config_dir: &Path, base_root: &Path) -> Vec<PathBuf> {
    let Some(branch) = crate::dev::get_active_worktree_branch(config_dir) else {
        return Vec::new();
    };
    let Some(worktree_root) =
        qol_dev_build::planning::worktree::find_git_worktree_by_branch(base_root, &branch)
    else {
        return Vec::new();
    };
    qol_workspace::discover_plugin_dirs(&worktree_root).unwrap_or_default()
}
