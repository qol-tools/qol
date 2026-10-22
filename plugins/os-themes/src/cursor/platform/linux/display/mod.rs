use anyhow::{bail, Result};

pub(super) mod x11;

pub(super) fn ensure_cursor_support() -> Result<()> {
    match qol_platform::linux_display_backend() {
        qol_platform::LinuxDisplayBackend::Wayland => {
            bail!("cursor effects require X11; Wayland is not supported yet")
        }
        qol_platform::LinuxDisplayBackend::X11 | qol_platform::LinuxDisplayBackend::Unknown => {
            Ok(())
        }
    }
}
