use super::lock::{open_lock_file, stale_lockfile};
use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

pub(super) fn acquire_operation_lock(
    plugins_dir: &Path,
    plugin_id: &str,
) -> Result<PluginOperationLock> {
    ensure_plugins_dir(plugins_dir)?;
    let path = lock_path(plugins_dir, plugin_id);

    match open_lock_file(&path) {
        Ok(mut file) => create_operation_lock(&path, plugin_id, &mut file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            reacquire_stale_lock(&path, plugin_id)
        }
        Err(error) => Err(error)
            .with_context(|| format!("Failed to acquire plugin operation lock {}", path.display())),
    }
}

#[derive(Debug)]
pub(super) struct PluginOperationLock {
    path: PathBuf,
}

impl Drop for PluginOperationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn ensure_plugins_dir(plugins_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(plugins_dir).with_context(|| {
        format!(
            "Failed to create plugins directory {}",
            plugins_dir.display()
        )
    })
}

pub(super) fn lock_path(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    plugins_dir.join(format!(".{}.lock", plugin_id))
}

fn create_operation_lock(
    path: &Path,
    plugin_id: &str,
    file: &mut std::fs::File,
) -> Result<PluginOperationLock> {
    write_lock_owner(file, plugin_id);
    Ok(PluginOperationLock {
        path: path.to_path_buf(),
    })
}

fn write_lock_owner(file: &mut std::fs::File, plugin_id: &str) {
    let _ = writeln!(file, "{} {}", std::process::id(), plugin_id);
}

fn reacquire_stale_lock(path: &Path, plugin_id: &str) -> Result<PluginOperationLock> {
    if !stale_lockfile(path, super::LOCKFILE_MAX_AGE) {
        anyhow::bail!("Plugin operation already in progress: {}", plugin_id);
    }

    let _ = std::fs::remove_file(path);
    let mut file = open_lock_file(path).with_context(|| {
        format!(
            "Failed to reacquire stale plugin operation lock {}",
            path.display()
        )
    })?;
    create_operation_lock(path, plugin_id, &mut file)
}
