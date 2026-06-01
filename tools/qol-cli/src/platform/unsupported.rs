use super::PlatformOps;
use anyhow::Result;

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
}
