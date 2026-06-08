use super::PlatformOps;
use anyhow::{anyhow, Result};

pub(crate) struct Platform;

impl PlatformOps for Platform {
    fn os_name(&self) -> &'static str {
        "unknown"
    }

    fn exe_name(&self, name: &str) -> String {
        name.to_string()
    }

    fn stop_qol_tray(&self) -> Result<()> {
        Ok(())
    }

    fn open_url(&self, _url: &str) {}

    fn copy_to_clipboard(&self, _text: &str) -> Result<()> {
        Err(anyhow!("clipboard not supported on this platform"))
    }
}
