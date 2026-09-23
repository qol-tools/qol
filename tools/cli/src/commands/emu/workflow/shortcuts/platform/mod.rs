use anyhow::Result;

use crate::commands::emu::BootedVm;

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
