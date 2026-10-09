use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// CLI Sessions owns parking; these subcommands forward to its installed binary,
/// so the park runner and the woken-tab close always come from the plugin qol-tray updates.
const PLUGIN_ID: &str = "qol-cli-sessions";

pub(super) fn forward(subcommand: &str, rest: &[OsString]) -> Result<()> {
    let binary = plugin_binary()?;
    let status = Command::new(&binary)
        .arg(subcommand)
        .args(rest)
        .status()
        .with_context(|| format!("failed to run {}", binary.display()))?;
    if !status.success() {
        bail!("`{PLUGIN_ID} {subcommand}` failed ({status})");
    }
    Ok(())
}

fn plugin_binary() -> Result<PathBuf> {
    let binary = qol_config::config_dir()
        .context("cannot resolve the qol-tray config directory")?
        .join("plugins")
        .join(PLUGIN_ID)
        .join(format!("{PLUGIN_ID}{}", std::env::consts::EXE_SUFFIX));
    if !binary.is_file() {
        bail!(
            "CLI Sessions is not installed: {} is missing",
            binary.display()
        );
    }
    Ok(binary)
}
