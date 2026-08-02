use anyhow::{bail, Result};
use std::os::unix::fs::PermissionsExt;

pub(in crate::commands::dev_bundle) fn source_is_executable(metadata: &std::fs::Metadata) -> bool {
    metadata.permissions().mode() & 0o111 != 0
}

pub(in crate::commands::dev_bundle) fn ensure_build_supported() -> Result<()> {
    bail!("Mint Cinnamon development bundles can currently only be built on Linux hosts")
}
