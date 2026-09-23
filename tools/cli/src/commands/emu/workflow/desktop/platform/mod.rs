use anyhow::Result;

use crate::commands::emu::BootedVm;

use super::super::DesktopGuestPlatform;

pub(in crate::commands::emu::workflow) mod linux;
mod macos;
mod windows;

pub(super) use super::super::Verdict;

pub(super) fn run(vm: &BootedVm, guest: DesktopGuestPlatform) -> Result<Verdict> {
    match guest {
        DesktopGuestPlatform::Linux => linux::run(vm),
        DesktopGuestPlatform::Macos => macos::run(vm),
        DesktopGuestPlatform::Windows => windows::run(vm),
    }
}
