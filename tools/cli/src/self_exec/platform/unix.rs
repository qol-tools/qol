use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::Path;

use anyhow::Result;

use super::SelfExecPlatform;

pub(crate) use super::termios::{apply_prior_termios, capture_prior_termios, restore_resumed_tty};

pub(super) struct Platform;

impl SelfExecPlatform for Platform {
    fn replace_process(&self, binary: &Path, args: &[OsString], env: (&str, &str)) -> Result<()> {
        let error = std::process::Command::new(binary)
            .args(args)
            .env(env.0, env.1)
            .exec();
        Err(error.into())
    }
}
