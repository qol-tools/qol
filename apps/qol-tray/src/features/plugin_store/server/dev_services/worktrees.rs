#![cfg(feature = "dev")]
use std::path::Path;

use super::super::types::WorktreeInfo;

pub(super) fn scan(manifest_dir: &Path) -> Vec<WorktreeInfo> {
    qol_dev_build::tray::list_worktrees(manifest_dir)
}

pub(super) fn resolve_git_branch(repo_dir: &Path) -> Option<String> {
    qol_dev_build::tray::resolve_git_branch(repo_dir)
}
