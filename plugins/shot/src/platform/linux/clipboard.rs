use anyhow::{anyhow, Context, Result};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn copy_image_to_clipboard(path: &Path) -> Result<()> {
    let wl_copy_error = match copy_image_with("wl-copy", &["--type", "image/png"], path) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    let xclip_error = match copy_image_with(
        "xclip",
        &["-selection", "clipboard", "-t", "image/png", "-i"],
        path,
    ) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };

    Err(anyhow!(
        "failed to copy image to clipboard; wl-copy: {wl_copy_error:#}; xclip: {xclip_error:#}"
    ))
}

pub fn copy_path_to_clipboard(path: &Path) -> Result<()> {
    let text = path.to_string_lossy();
    let wl_copy_error = match copy_text_with("wl-copy", &[], &text) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    let xclip_error = match copy_text_with("xclip", &["-selection", "clipboard"], &text) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };

    Err(anyhow!(
        "failed to copy path to clipboard; wl-copy: {wl_copy_error:#}; xclip: {xclip_error:#}"
    ))
}

fn copy_text_with(program: &str, args: &[&str], text: &str) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to run {program}"))?;

    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to open {program} stdin"))?
        .write_all(text.as_bytes())
        .with_context(|| format!("failed to write to {program}"))?;

    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {program}"))?;
    if status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "{program} text clipboard copy exited with {status}"
    ))
}

fn copy_image_with(program: &str, args: &[&str], path: &Path) -> Result<()> {
    let file = File::open(path).with_context(|| {
        format!(
            "failed to open screenshot for clipboard: {}",
            path.display()
        )
    })?;
    let status = Command::new(program)
        .args(args)
        .stdin(Stdio::from(file))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to run {program}"))?;

    if status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "{program} image clipboard copy exited with {status}"
    ))
}
