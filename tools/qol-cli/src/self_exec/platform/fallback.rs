use std::ffi::OsString;
use std::path::Path;

use anyhow::{bail, Result};

use super::SelfExecPlatform;

pub(super) struct Platform;

impl SelfExecPlatform for Platform {
    fn replace_process(&self, _binary: &Path, _args: &[OsString], _tray_pid: u32) -> Result<()> {
        bail!("replacing the qol process is not supported on this platform")
    }
}
