use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use super::AutostartOps;

pub(crate) struct Platform;

impl AutostartOps for Platform {
    fn read_target(&self) -> Result<Option<PathBuf>> {
        bail!("autostart artifact not implemented on this OS")
    }

    fn write_target(&self, _binary: &Path) -> Result<()> {
        bail!("autostart artifact not implemented on this OS")
    }

    fn autostart_path(&self) -> Result<PathBuf> {
        bail!("autostart artifact not implemented on this OS")
    }
}
