use std::ffi::OsString;
use std::path::Path;

use anyhow::{bail, Result};

use super::SelfExecPlatform;

pub(super) struct Platform;

impl SelfExecPlatform for Platform {
    fn replace_process(
        &self,
        _binary: &Path,
        _args: &[OsString],
        _env: (&str, &str),
    ) -> Result<()> {
        bail!("replacing the qol process is not supported on this platform")
    }
}

pub(crate) fn apply_prior_termios() {}

pub(crate) fn capture_prior_termios() {}

pub(crate) fn restore_resumed_tty() {}
