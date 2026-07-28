use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use super::PayloadPlatform;

pub(super) struct Platform;

impl PayloadPlatform for Platform {
    fn set_file_mode(&self, path: &Path, executable: bool) -> Result<()> {
        let mode = if executable { 0o555 } else { 0o444 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed to set payload mode on {}", path.display()))
    }

    fn make_tree_read_only(&self, root: &Path) -> Result<()> {
        for path in super::super::directories_deepest_first(root)? {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o555))
                .with_context(|| format!("failed to make {} read-only", path.display()))?;
        }
        Ok(())
    }

    fn make_tree_writable(&self, root: &Path) -> Result<()> {
        for path in super::super::directories_deepest_first(root)? {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn make_file_writable(&self, path: &Path) -> Result<()> {
        let mut permissions = path.metadata()?.permissions();
        permissions.set_mode(permissions.mode() | 0o200);
        fs::set_permissions(path, permissions)?;
        Ok(())
    }
}
