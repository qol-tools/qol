use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

pub(in crate::exec) fn replace_process(program: &Path, args: &[String], cwd: &Path) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("failed to run {}", program.display()))?;
    std::process::exit(status.code().unwrap_or(1));
}
