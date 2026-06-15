use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::Platform;
#[cfg(target_os = "macos")]
pub(crate) use macos::Platform;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use unsupported::Platform;
#[cfg(target_os = "windows")]
pub(crate) use windows::Platform;

pub(crate) trait PlatformOps {
    fn os_name(&self) -> &'static str;
    fn exe_name(&self, name: &str) -> String;
    fn stop_qol_tray(&self) -> Result<()>;
    fn open_url(&self, url: &str);
    fn open_path(&self, dir: &Path);
    fn copy_to_clipboard(&self, text: &str) -> Result<()>;
}

pub(super) fn pipe_to_clipboard(program: &str, args: &[&str], text: &str) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("could not run {program}"))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .context("clipboard command has no stdin")?;
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("{program} exited with {status}"))
    }
}
