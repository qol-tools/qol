use anyhow::{anyhow, Context, Result};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use super::swift::{ensure_swift_helper, CLIPBOARD_WRITER_HELPER, CLIPBOARD_WRITER_SWIFT};

pub fn copy_image_to_clipboard(path: &Path) -> Result<()> {
    let helper = ensure_swift_helper(
        "clipboard-writer",
        CLIPBOARD_WRITER_SWIFT,
        CLIPBOARD_WRITER_HELPER,
    )
    .context("failed to install embedded clipboard writer helper")?;

    let output = Command::new(&helper)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .inspect_err(|_| {
            let _ = fs::remove_file(&helper);
        })
        .context("failed to start compiled macOS clipboard writer")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "macOS clipboard writer exited with {}: {}",
        output.status,
        stderr.trim()
    ))
}

pub fn copy_path_to_clipboard(path: &Path) -> Result<()> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to run pbcopy")?;

    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to open pbcopy stdin"))?
        .write_all(path.to_string_lossy().as_bytes())
        .context("failed to write to pbcopy")?;

    let status = child.wait().context("failed to wait for pbcopy")?;
    if status.success() {
        return Ok(());
    }

    Err(anyhow!("pbcopy exited with {status}"))
}
