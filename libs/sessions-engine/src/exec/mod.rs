use std::path::Path;

use anyhow::Result;

mod platform;

pub(super) fn replace_process(program: &Path, args: &[String], cwd: &Path) -> Result<()> {
    platform::replace_process(program, args, cwd)
}
