use anyhow::{bail, Result};

use crate::commands::emu::BootedVm;

use super::Verdict;

pub(super) fn run(_vm: &BootedVm) -> Result<Verdict> {
    bail!("alt-tab-storm is not implemented for Windows desktop guests")
}

pub(super) fn run_performance(_vm: &BootedVm) -> Result<Verdict> {
    bail!("alt-tab-performance is not implemented for Windows desktop guests")
}
