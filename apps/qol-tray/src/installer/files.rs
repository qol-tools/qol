use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

pub(super) fn ensure_plugin_dir() -> Result<PathBuf> {
    let plugins_dir = crate::paths::plugins_dir()?;
    fs::create_dir_all(&plugins_dir).with_context(|| {
        format!(
            "Failed to create plugins directory {}",
            plugins_dir.display()
        )
    })?;
    Ok(plugins_dir)
}
