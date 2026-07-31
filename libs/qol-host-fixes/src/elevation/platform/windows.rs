use anyhow::{bail, Result};

pub(crate) fn available() -> bool {
    false
}

pub(crate) fn run(_label: &str, _script: &str, _args: &[String]) -> Result<()> {
    bail!("privileged host fixes have no Windows backend yet")
}
