use super::PlatformOps;
use anyhow::{anyhow, Result};
use std::process::Command;

pub(crate) struct Platform;

impl PlatformOps for Platform {
    fn os_name(&self) -> &'static str {
        "linux"
    }

    fn exe_name(&self, name: &str) -> String {
        name.to_string()
    }

    fn stop_qol_tray(&self) -> Result<()> {
        let _ = Command::new("pkill").args(["-x", "qol-tray"]).status();
        Ok(())
    }

    fn qol_tray_running(&self) -> bool {
        let Ok(output) = Command::new("pgrep").args(["-x", "qol-tray"]).output() else {
            return false;
        };
        output
            .stdout
            .split(|byte| *byte == b'\n')
            .filter_map(|line| std::str::from_utf8(line).ok())
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .any(|pid| qol_process::is_pid_alive(pid) && !qol_process::is_pid_zombie(pid))
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<()> {
        super::pipe_to_clipboard("wl-copy", &[], text)
            .or_else(|_| super::pipe_to_clipboard("xclip", &["-selection", "clipboard"], text))
            .or_else(|_| super::pipe_to_clipboard("xsel", &["--clipboard", "--input"], text))
            .map_err(|_| anyhow!("no clipboard tool found (install wl-copy, xclip, or xsel)"))
    }
}
