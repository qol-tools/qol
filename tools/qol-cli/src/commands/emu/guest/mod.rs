mod debian;

pub(crate) use debian::DebianNocloud;

use super::serial::SerialClient;
use anyhow::{bail, Result};

static DEBIAN_NOCLOUD: debian::DebianNocloud = debian::DebianNocloud;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestAdapter {
    DebianNocloud,
    MintCinnamon,
}

impl GuestAdapter {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "debian-nocloud" => Some(Self::DebianNocloud),
            "mint-cinnamon" => Some(Self::MintCinnamon),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DebianNocloud => "debian-nocloud",
            Self::MintCinnamon => "mint-cinnamon",
        }
    }

    pub(crate) fn guest(self) -> Result<&'static dyn GuestOs> {
        match self {
            Self::DebianNocloud => Ok(&DEBIAN_NOCLOUD),
            Self::MintCinnamon => bail!(
                "guest adapter `mint-cinnamon` is prepared but not ready: serial login, removable-media discovery, reboot relogin, and trace locations are not verified"
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
            ("mint-cinnamon", Some(GuestAdapter::MintCinnamon)),
            ("mint", None),
            ("", None),
        ];
        for (id, expected) in cases {
            assert_eq!(GuestAdapter::parse(id), expected, "adapter: {id}");
        }

        assert!(GuestAdapter::DebianNocloud.guest().is_ok());
        let error = GuestAdapter::MintCinnamon.guest().err().unwrap();
        assert!(error.to_string().contains("prepared but not ready"));
    }
}
