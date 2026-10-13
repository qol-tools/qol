use super::RestartPlatformOps;
use std::ffi::OsString;
use std::path::Path;

pub(super) struct Platform;

impl RestartPlatformOps for Platform {
    fn binary_name() -> &'static str {
        "qol-tray.exe"
    }

    fn exec_restart(binary: &Path) -> Result<(), String> {
        let args: Vec<OsString> = std::env::args_os().skip(1).collect();
        Err(crate::relaunch::spawn_successor_and_exit(binary, &args).to_string())
    }
}
