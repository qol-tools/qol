use anyhow::Result;

use crate::commands::emu::BootedVm;

use super::{DesktopGuestPlatform, Verdict};

mod platform;

pub(super) fn run(vm: &BootedVm, guest: DesktopGuestPlatform) -> Result<Verdict> {
    platform::run(vm, guest)
}

pub(super) fn run_performance(vm: &BootedVm, guest: DesktopGuestPlatform) -> Result<Verdict> {
    platform::run_performance(vm, guest)
}
