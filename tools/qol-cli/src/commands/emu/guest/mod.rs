mod debian;

pub(crate) use debian::DebianNocloud;

use super::serial::SerialClient;
use anyhow::Result;

pub(crate) trait GuestOs {
    fn ensure_root_shell(&self, serial: &mut SerialClient) -> Result<()>;
    fn launch_qol_from_stick(&self, serial: &mut SerialClient) -> Result<()>;
    fn reboot_and_relogin(&self, serial: &mut SerialClient) -> Result<()>;
    fn list_qol_traces(&self, serial: &mut SerialClient) -> Result<Vec<String>>;
}
