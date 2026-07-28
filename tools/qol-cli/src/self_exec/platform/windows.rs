use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context, Result};

use super::SelfExecPlatform;

pub(super) struct Platform;

impl SelfExecPlatform for Platform {
    fn replace_process(&self, binary: &Path, args: &[OsString], tray_pid: u32) -> Result<()> {
        std::process::Command::new(binary)
            .args(args)
            .env(crate::self_exec::RESUME_TRAY_PID_ENV, tray_pid.to_string())
            .spawn()
            .context("failed to spawn successor qol process")?;
        std::process::exit(0);
    }
}
