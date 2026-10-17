use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

pub(in crate::exec) fn replace_process(program: &Path, args: &[String], cwd: &Path) -> Result<()> {
    let error = Command::new(program).args(args).current_dir(cwd).exec();
    Err(error).with_context(|| format!("failed to run {}", program.display()))
}
