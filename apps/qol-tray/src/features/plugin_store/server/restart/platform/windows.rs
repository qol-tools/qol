use std::path::Path;

pub(super) fn binary_name() -> &'static str {
    "qol-tray.exe"
}

pub(super) fn exec_restart(binary: &Path) -> Result<(), String> {
    std::process::Command::new(binary)
        .args(std::env::args_os().skip(1))
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}
