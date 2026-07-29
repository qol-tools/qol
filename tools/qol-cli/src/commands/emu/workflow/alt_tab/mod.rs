use anyhow::Result;

use crate::commands::emu::BootedVm;

use super::{DesktopGuestPlatform, Verdict};

mod platform;

pub(super) fn run(vm: &BootedVm, guest: DesktopGuestPlatform) -> Result<Verdict> {
    platform::run(vm, guest)
}
