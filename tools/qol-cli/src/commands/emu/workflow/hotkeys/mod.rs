use anyhow::Result;

use crate::commands::emu::{qmp, BootedVm};
use qol_dev_guest::GuestControlClient;

use super::{DesktopGuestPlatform, Verdict};

mod platform;

pub(super) fn run(vm: &BootedVm, guest: DesktopGuestPlatform) -> Result<Verdict> {
    platform::run(vm, guest)
}

pub(super) fn require_passthrough(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
) -> Result<()> {
    platform::require_passthrough(guest, qmp)
}
