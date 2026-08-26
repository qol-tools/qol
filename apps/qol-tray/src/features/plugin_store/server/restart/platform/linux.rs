use super::RestartPlatformOps;
use std::os::unix::process::CommandExt;
use std::path::Path;

pub(super) struct Platform;

impl RestartPlatformOps for Platform {
    fn binary_name() -> &'static str {
        "qol-tray"
    }

    fn exec_restart(binary: &Path) -> Result<(), String> {
        let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
        // exec destroys the uinput keyboard with whatever is down still
        // latched in the X server; release it first or that key autorepeats
        // until reboot.
        crate::hotkeys::release_held_keys();
        crate::lifeline_handoff::prepare_for_exec();
        let error = std::process::Command::new(binary).args(&args).exec();
        Err(error.to_string())
    }
}
