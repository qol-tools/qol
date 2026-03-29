use anyhow::Result;
use std::path::Path;

pub(crate) fn open_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        anyhow::bail!("Directory does not exist");
    }
    std::process::Command::new("open").arg(dir).spawn()?;
    Ok(())
}
