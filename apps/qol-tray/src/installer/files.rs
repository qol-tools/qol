use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

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

pub(super) fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)
        .with_context(|| format!("Failed to create directory {}", target.display()))?;

    for entry in fs::read_dir(source)
        .with_context(|| format!("Failed to read directory {}", source.display()))?
    {
        let entry =
            entry.with_context(|| format!("Failed to read entry in {}", source.display()))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        copy_entry(&source_path, &target_path)?;
    }

    Ok(())
}

fn copy_entry(source: &Path, target: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Failed to read metadata for {}", source.display()))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return super::platform::copy_symlink(source, target);
    }
    if file_type.is_dir() {
        return copy_dir_recursive(source, target);
    }
    if file_type.is_file() {
        return copy_file(source, target);
    }
    Ok(())
}

fn copy_file(source: &Path, target: &Path) -> Result<()> {
    fs::copy(source, target).with_context(|| {
        format!(
            "Failed to copy {} to {}",
            source.display(),
            target.display()
        )
    })?;
    super::platform::on_file_copied(source, target)
}
