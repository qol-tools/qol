use anyhow::{bail, Result};
use std::path::Path;

pub(crate) fn file_mode(_metadata: &std::fs::Metadata) -> Result<u32> {
    bail!("resident bundle preparation requires Unix permission metadata")
}

pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> Result<()> {
    bail!(
        "atomic no-replace snapshot publication is unsupported on Windows: {} -> {}",
        from.display(),
        to.display()
    )
}
