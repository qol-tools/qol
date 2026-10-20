use anyhow::{bail, Context, Result};

use crate::lock::Availability;

pub fn lock_screen() -> Result<()> {
    let mut command = qol_process::system_command("rundll32.exe");
    command.arg("user32.dll,LockWorkStation");
    qol_process::hide_console_window(&mut command);
    let status = command.status().context("failed to run rundll32.exe")?;
    if !status.success() {
        bail!("LockWorkStation exited with {status}");
    }
    Ok(())
}

pub fn availability() -> Availability {
    Availability::Ready("Windows LockWorkStation will lock the screen.".into())
}
