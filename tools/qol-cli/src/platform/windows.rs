use super::PlatformOps;
use anyhow::Result;
use std::process::Command;

pub(crate) struct Platform;

impl PlatformOps for Platform {
    fn os_name(&self) -> &'static str {
        "windows"
    }

    fn exe_name(&self, name: &str) -> String {
        format!("{name}.exe")
    }

    fn stop_qol_tray(&self) -> Result<()> {
        let _ = Command::new("taskkill")
            .args(["/IM", "qol-tray.exe", "/F"])
            .status();
        Ok(())
    }
}
