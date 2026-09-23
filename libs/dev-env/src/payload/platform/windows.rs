use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::PayloadPlatform;

pub(super) struct Platform;

impl PayloadPlatform for Platform {
    fn set_file_mode(&self, path: &Path, _executable: bool) -> Result<()> {
        let mut permissions = path.metadata()?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to make payload file read-only: {}", path.display()))
    }

    fn make_tree_read_only(&self, root: &Path) -> Result<()> {
        for path in super::super::directories_deepest_first(root)? {
            let mut permissions = path.metadata()?.permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&path, permissions)?;
        }
        Ok(())
    }

    fn make_tree_writable(&self, root: &Path) -> Result<()> {
        for path in super::super::directories_deepest_first(root)? {
            clear_readonly(&path)?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn make_file_writable(&self, path: &Path) -> Result<()> {
        clear_readonly(path)
    }
}

fn clear_readonly(path: &Path) -> Result<()> {
    qol_fs::prepare_file_removal(path)
        .with_context(|| format!("failed to make payload path writable: {}", path.display()))
}
