mod debian;

pub(crate) use debian::DebianNocloud;

use super::serial::SerialClient;
use anyhow::Result;

pub(crate) trait GuestOs {
    fn ensure_root_shell(&self, serial: &mut SerialClient) -> Result<()>;
}
