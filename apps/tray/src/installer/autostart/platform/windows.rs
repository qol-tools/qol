use anyhow::Result;
use std::path::{Path, PathBuf};

use super::{AutostartOps, AutostartState};
use crate::installer::platform::windows::run_key;

pub(super) struct Platform;

impl AutostartOps for Platform {
    fn read_state(&self) -> Result<AutostartState> {
        run_key::read_state()
    }

    fn write_target(&self, binary: &Path) -> Result<()> {
        run_key::write(binary)
    }

    fn autostart_path(&self) -> Result<PathBuf> {
        Ok(run_key::location())
    }
}
