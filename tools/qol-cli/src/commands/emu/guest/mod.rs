mod debian;

pub(crate) use debian::DebianNocloud;

use super::serial::SerialClient;
use anyhow::Result;
use serde_json::Value;

pub(crate) trait GuestOs {
    fn ensure_root_shell(&self, serial: &mut SerialClient) -> Result<()>;
    fn verify_uninstall_from_stick(&self, serial: &mut SerialClient) -> Result<Value>;
    fn reboot_and_relogin(&self, serial: &mut SerialClient) -> Result<()>;
    fn list_qol_traces(&self, serial: &mut SerialClient) -> Result<Vec<String>>;
}
