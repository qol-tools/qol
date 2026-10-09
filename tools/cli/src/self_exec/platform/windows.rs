use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context, Result};

use super::SelfExecPlatform;

pub(super) struct Platform;

impl SelfExecPlatform for Platform {
    fn replace_process(&self, binary: &Path, args: &[OsString], env: (&str, &str)) -> Result<()> {
        std::process::Command::new(binary)
            .args(args)
            .env(env.0, env.1)
            .spawn()
            .context("failed to spawn successor qol process")?;
        std::process::exit(0);
    }
}

pub(crate) fn apply_prior_termios() {}

pub(crate) fn capture_prior_termios() {}

pub(crate) fn restore_resumed_tty() {}
