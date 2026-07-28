use super::super::Platform;
use anyhow::{bail, Result};

pub(super) fn current() -> Result<Platform> {
    bail!("qol check is not verified on windows")
}
