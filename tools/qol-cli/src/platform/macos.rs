use super::PlatformOps;
use anyhow::Result;
use std::process::Command;

pub(crate) struct Platform;

impl PlatformOps for Platform {
    fn os_name(&self) -> &'static str {
        "macos"
    }

    fn exe_name(&self, name: &str) -> String {
        name.to_string()
    }

    fn stop_qol_tray(&self) -> Result<()> {
        let _ = Command::new("pkill").args(["-x", "qol-tray"]).status();
        Ok(())
    }

    fn qol_tray_running(&self) -> bool {
        Command::new("pgrep")
            .args(["-x", "qol-tray"])
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<()> {
        super::pipe_to_clipboard("pbcopy", &[], text)
    }
}
