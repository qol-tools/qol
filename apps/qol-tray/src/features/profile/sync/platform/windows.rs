use anyhow::Result;
use std::path::Path;

pub(crate) fn open_dir(dir: &Path) -> Result<()> {
    open_path(dir)
}

pub(crate) fn open_path(path: &Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("Path does not exist");
    }
    std::process::Command::new("explorer").arg(path).spawn()?;
    Ok(())
}
