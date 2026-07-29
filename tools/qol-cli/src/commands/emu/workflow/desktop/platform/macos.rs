use anyhow::{bail, Result};

use crate::commands::emu::BootedVm;

use super::Verdict;

pub(super) fn run(_vm: &BootedVm) -> Result<Verdict> {
    bail!("qol-shot-capture is not implemented for macOS desktop guests")
}
