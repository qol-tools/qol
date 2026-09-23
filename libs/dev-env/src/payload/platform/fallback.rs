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
        anyhow::bail!(
            "making payload trees writable is unsupported on this platform: {}",
            root.display()
        )
    }

    #[cfg(test)]
    fn make_file_writable(&self, path: &Path) -> Result<()> {
        anyhow::bail!(
            "making payload files writable is unsupported on this platform: {}",
            path.display()
        )
    }
}
