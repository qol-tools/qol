use anyhow::{bail, Result};

use crate::commands::emu::BootedVm;

use super::Verdict;

pub(super) fn run(_vm: &BootedVm) -> Result<Verdict> {
    bail!("hotkey-storm is not implemented for Windows desktop guests")
}
