use std::os::unix::process::CommandExt;
use std::path::Path;

pub(super) fn exec_restart(binary: &Path) -> Result<(), String> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    crate::lifeline_handoff::prepare_for_exec();
    let error = std::process::Command::new(binary).args(&args).exec();
    Err(error.to_string())
}
