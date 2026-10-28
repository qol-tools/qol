use std::path::Path;

use anyhow::Result;

pub(crate) fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    std::fs::write(path, contents)?;
    Ok(())
}
