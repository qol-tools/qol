use super::PlatformOps;
use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::{Command, Stdio};

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

    fn open_url(&self, url: &str) {
        let _ = Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    fn open_path(&self, dir: &Path) {
        let (program, args) = open_path_argv(dir);
        let _ = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<()> {
        super::pipe_to_clipboard("wl-copy", &[], text)
            .or_else(|_| super::pipe_to_clipboard("xclip", &["-selection", "clipboard"], text))
            .or_else(|_| super::pipe_to_clipboard("xsel", &["--clipboard", "--input"], text))
            .map_err(|_| anyhow!("no clipboard tool found (install wl-copy, xclip, or xsel)"))
    }
}

fn open_path_argv(dir: &Path) -> (&'static str, Vec<String>) {
    ("xdg-open", vec![dir.display().to_string()])
}
