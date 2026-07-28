use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::Path;

use anyhow::Result;

use super::SelfExecPlatform;

pub(super) struct Platform;

impl SelfExecPlatform for Platform {
    fn replace_process(&self, binary: &Path, args: &[OsString], tray_pid: u32) -> Result<()> {
        let error = std::process::Command::new(binary)
            .args(args)
            .env(crate::self_exec::RESUME_TRAY_PID_ENV, tray_pid.to_string())
            .exec();
        Err(error.into())
    }
}
