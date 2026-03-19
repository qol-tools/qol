use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn ensure_parent_dir(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory {}", parent.display()))
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let content = fs::read_to_string(path)?;
    let value = serde_json::from_str(&content)?;
    Ok(value)
}

pub(crate) fn load_json_or_default<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    read_json(path)
}

pub(crate) fn write_pretty_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    ensure_parent_dir(path)?;
    let content = serde_json::to_string_pretty(value)?;
    fs::write(path, content)?;
    Ok(())
}

pub(crate) fn canonical_or_original(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
