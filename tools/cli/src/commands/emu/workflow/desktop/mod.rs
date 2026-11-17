use anyhow::Result;

use crate::commands::emu::BootedVm;

use super::{DesktopGuestPlatform, Verdict};

mod platform;

pub(in crate::commands::emu::workflow) use platform::linux;

pub(super) fn run(vm: &BootedVm, guest: DesktopGuestPlatform) -> Result<Verdict> {
    platform::run(vm, guest)
}
