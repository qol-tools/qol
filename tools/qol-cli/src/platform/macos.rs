use super::PlatformOps;
use anyhow::Result;
use std::path::Path;
use std::process::{Command, Stdio};

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

    fn open_url(&self, url: &str) {
        let _ = Command::new("open")
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
        super::pipe_to_clipboard("pbcopy", &[], text)
    }
}

fn open_path_argv(dir: &Path) -> (&'static str, Vec<String>) {
    ("open", vec![dir.display().to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn open_path_argv_uses_open_with_dir_argument() {
        let (program, args) = open_path_argv(Path::new("/a/b/emu"));
        assert_eq!(program, "open", "program");
        assert_eq!(args, vec!["/a/b/emu".to_string()], "args");
    }
}
