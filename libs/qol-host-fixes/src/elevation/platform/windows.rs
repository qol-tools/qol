use anyhow::{bail, Result};
use std::ffi::OsString;
use std::path::Path;

pub(crate) fn available() -> bool {
    false
}

pub(crate) fn run(_label: &str, _script: &str, _args: &[String]) -> Result<()> {
    bail!("privileged host fixes have no Windows backend yet")
}

pub(crate) fn spawn(
    _label: &str,
    _program: &Path,
    _args: &[OsString],
) -> Result<std::process::Child> {
    bail!("privileged host fixes have no Windows backend yet")
}
