mod debian;

pub(crate) use debian::DebianNocloud;

use super::serial::SerialClient;
use super::strategy::{DesktopStrategy, GuestPlan, GuestStrategy};
use anyhow::{bail, Result};

static DEBIAN_NOCLOUD: debian::DebianNocloud = debian::DebianNocloud;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestAdapter {
    DebianNocloud,
    MacosDesktop,
    MintCinnamon,
    WindowsDesktop,
}

impl GuestAdapter {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "debian-nocloud" => Some(Self::DebianNocloud),
            "macos-desktop" => Some(Self::MacosDesktop),
            "mint-cinnamon" => Some(Self::MintCinnamon),
            "windows-desktop" => Some(Self::WindowsDesktop),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DebianNocloud => "debian-nocloud",
            Self::MacosDesktop => "macos-desktop",
            Self::MintCinnamon => "mint-cinnamon",
            Self::WindowsDesktop => "windows-desktop",
        }
    }

    pub(crate) fn plan(self) -> GuestPlan {
        match self {
            Self::DebianNocloud => GuestPlan::new(GuestStrategy::DebianNocloud, None),
            Self::MacosDesktop => {
                GuestPlan::new(GuestStrategy::Macos, Some(DesktopStrategy::Macos))
            }
            Self::MintCinnamon => {
                GuestPlan::new(GuestStrategy::MintCinnamon, Some(DesktopStrategy::Linux))
            }
            Self::WindowsDesktop => {
                GuestPlan::new(GuestStrategy::Windows, Some(DesktopStrategy::Windows))
            }
        }
    }

    pub(crate) fn guest(self) -> Result<&'static dyn GuestOs> {
        match self.plan().guest_strategy() {
            GuestStrategy::DebianNocloud => Ok(&DEBIAN_NOCLOUD),
            GuestStrategy::Macos => bail!(
                "guest strategy `macos` is not available yet; the Apple Virtualization.framework guest backend is not implemented"
            ),
            GuestStrategy::MintCinnamon | GuestStrategy::Windows => bail!(
                "guest strategy `{}` does not implement the serial GuestOs contract",
                self.as_str()
            ),
        }
    }
}

pub(crate) trait GuestOs {
    fn ensure_root_shell(&self, serial: &mut SerialClient) -> Result<()>;
    fn launch_qol_from_stick(&self, serial: &mut SerialClient) -> Result<()>;
    fn reboot_and_relogin(&self, serial: &mut SerialClient) -> Result<()>;
    fn list_qol_traces(&self, serial: &mut SerialClient) -> Result<Vec<String>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_registry_distinguishes_ready_prepared_and_unknown_entries() {
        let cases = [
            ("debian-nocloud", Some(GuestAdapter::DebianNocloud)),
            ("macos-desktop", Some(GuestAdapter::MacosDesktop)),
            ("mint-cinnamon", Some(GuestAdapter::MintCinnamon)),
            ("windows-desktop", Some(GuestAdapter::WindowsDesktop)),
            ("mint", None),
            ("", None),
        ];
        for (id, expected) in cases {
            assert_eq!(GuestAdapter::parse(id), expected, "adapter: {id}");
        }

        assert!(GuestAdapter::DebianNocloud.guest().is_ok());
        let macos_error = GuestAdapter::MacosDesktop.guest().err().unwrap();
        assert!(macos_error.to_string().contains("not available yet"));
        for adapter in [GuestAdapter::MintCinnamon, GuestAdapter::WindowsDesktop] {
            let error = adapter.guest().err().unwrap();
            assert!(error.to_string().contains("serial GuestOs contract"));
        }
    }

    #[test]
    fn adapter_plan_separates_guest_and_desktop_strategies() {
        assert_eq!(
            GuestAdapter::DebianNocloud.plan().guest_strategy(),
            GuestStrategy::DebianNocloud
        );
        assert_eq!(
            GuestAdapter::MacosDesktop.plan().desktop().unwrap(),
            DesktopStrategy::Macos
        );
        assert!(GuestAdapter::DebianNocloud.plan().desktop().is_err());
    }
}
