mod platform;

use anyhow::{bail, Result};
use std::ffi::OsString;
use std::path::Path;

pub(crate) fn validate(path: Option<&Path>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    if !path.is_absolute() {
        bail!("--usb-host must be an absolute USB device path");
    }
    platform::validate_device(path)
}

pub(crate) fn acquire(path: Option<&Path>, run_id: &str) -> Result<Option<UsbHostLease>> {
    let Some(path) = path else {
        return Ok(None);
    };
    validate(Some(path))?;
    platform::spawn(path, run_id).map(|lease| Some(UsbHostLease { inner: Some(lease) }))
}

pub(crate) fn run_helper(args: &[OsString]) -> Result<()> {
    platform::run_helper(args)
}

pub(crate) struct UsbHostLease {
    inner: Option<platform::LeaseConnection>,
}

impl UsbHostLease {
    pub(crate) fn release(&mut self) -> Result<()> {
        if let Some(mut inner) = self.inner.take() {
            return inner.release();
        }
        Ok(())
    }
}

impl Drop for UsbHostLease {
    fn drop(&mut self) {
        if let Err(error) = self.release() {
            eprintln!("Bluetooth USB host lease cleanup failed: {error:#}");
        }
    }
}
