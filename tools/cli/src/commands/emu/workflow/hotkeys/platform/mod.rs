use anyhow::Result;
use qol_dev_guest::GuestControlClient;

use crate::commands::emu::{qmp, BootedVm};

use super::super::DesktopGuestPlatform;

mod linux;
mod macos;
mod windows;

pub(super) use crate::commands::emu::workflow::desktop::platform::linux as desktop;
pub(super) use crate::commands::emu::workflow::Verdict;

pub(super) fn run(vm: &BootedVm, guest: DesktopGuestPlatform) -> Result<Verdict> {
    match guest {
        DesktopGuestPlatform::Linux => linux::run(vm),
        DesktopGuestPlatform::Macos => macos::run(vm),
        DesktopGuestPlatform::Windows => windows::run(vm),
    }
}

pub(super) fn require_passthrough(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
) -> Result<()> {
    linux::require_passthrough_keys(guest, qmp)
}
