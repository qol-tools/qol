use anyhow::{bail, Result};

use crate::commands::emu::BootedVm;

use super::Verdict;

pub(super) fn run(_vm: &BootedVm) -> Result<Verdict> {
    bail!("launcher-storm is not implemented for macOS desktop guests")
}
