use anyhow::{bail, Result};

mod machine;

pub(crate) use machine::{resolve_machine_strategy, MachineBackend};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GuestStrategy {
    DebianNocloud,
    MintCinnamon,
    Macos,
    Windows,
}

impl GuestStrategy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DebianNocloud => "debian-nocloud",
            Self::MintCinnamon => "mint-cinnamon",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DesktopStrategy {
    Linux,
    Macos,
    Windows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GuestPlan {
    guest: GuestStrategy,
    desktop: Option<DesktopStrategy>,
}

impl GuestPlan {
    pub(crate) fn new(guest: GuestStrategy, desktop: Option<DesktopStrategy>) -> Self {
        Self { guest, desktop }
    }

    pub(crate) fn guest_strategy(self) -> GuestStrategy {
        self.guest
    }

    pub(crate) fn desktop(self) -> Result<DesktopStrategy> {
        self.desktop.ok_or_else(|| {
            anyhow::anyhow!(
                "guest strategy `{}` does not provide the desktop workflow contract",
                self.guest.as_str()
            )
        })
    }

    pub(crate) fn serial_guest(self) -> Result<()> {
        if self.guest == GuestStrategy::DebianNocloud {
            return Ok(());
        }
        bail!(
            "guest strategy `{}` does not provide the serial workflow contract",
            self.guest.as_str()
        )
    }
}
