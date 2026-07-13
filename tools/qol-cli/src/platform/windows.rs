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

    fn qol_tray_running(&self) -> bool {
        let Ok(output) = Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq qol-tray.exe", "/NH"])
            .output()
        else {
            return false;
        };
        String::from_utf8_lossy(&output.stdout)
            .to_ascii_lowercase()
            .contains("qol-tray.exe")
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<()> {
        super::pipe_to_clipboard("clip", &[], text)
    }
}
