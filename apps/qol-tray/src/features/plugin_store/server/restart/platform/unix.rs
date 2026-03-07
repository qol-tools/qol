use std::os::unix::process::CommandExt;
use std::path::Path;

pub(super) fn binary_name() -> &'static str {
    "qol-tray"
}

pub(super) fn exec_restart(binary: &Path) -> Result<(), String> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let error = std::process::Command::new(binary).args(&args).exec();
    Err(error.to_string())
}
