use anyhow::Result;
use std::path::{Path, PathBuf};

use super::AutostartOps;
use crate::installer::platform::windows::run_key;

pub(super) struct Platform;

impl AutostartOps for Platform {
    fn read_target(&self) -> Result<Option<PathBuf>> {
        run_key::read()
    }

    fn write_target(&self, binary: &Path) -> Result<()> {
        run_key::write(binary)
    }

    fn autostart_path(&self) -> Result<PathBuf> {
        Ok(run_key::location())
    }
}
