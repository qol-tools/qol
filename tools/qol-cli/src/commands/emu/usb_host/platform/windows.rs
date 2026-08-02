use anyhow::{bail, Result};
use std::ffi::OsString;
use std::path::Path;

pub(crate) struct LeaseConnection;

impl LeaseConnection {
    pub(crate) fn release(&mut self) -> Result<()> {
        Ok(())
    }
}

pub(crate) fn validate_device(_path: &Path) -> Result<()> {
    bail!("--usb-host is supported only on Linux")
}

pub(crate) fn spawn(_path: &Path, _run_id: &str) -> Result<LeaseConnection> {
    bail!("--usb-host is supported only on Linux")
}

pub(crate) fn run_helper(_args: &[OsString]) -> Result<()> {
    bail!("the USB host lease helper is supported only on Linux")
}
