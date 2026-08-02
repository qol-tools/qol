use anyhow::{bail, Result};

pub(in crate::commands::dev_bundle) fn source_is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

pub(in crate::commands::dev_bundle) fn ensure_build_supported() -> Result<()> {
    bail!("Mint Cinnamon development bundles can currently only be built on Linux hosts")
}
