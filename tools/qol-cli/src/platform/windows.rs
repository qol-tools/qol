use super::PlatformOps;
use anyhow::Result;
use std::path::Path;
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

    fn open_url(&self, url: &str) {
        let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    }

    fn open_path(&self, dir: &Path) {
        let (program, args) = open_path_argv(dir);
        let _ = Command::new(program).args(args).spawn();
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<()> {
        super::pipe_to_clipboard("clip", &[], text)
    }
}

fn open_path_argv(dir: &Path) -> (&'static str, Vec<String>) {
    ("explorer", vec![dir.display().to_string()])
}
