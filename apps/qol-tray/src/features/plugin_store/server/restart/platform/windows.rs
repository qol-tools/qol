use std::path::Path;

pub(super) fn binary_name() -> &'static str {
    "qol-tray.exe"
}

pub(super) fn spawn_delayed(binary: &Path) -> Result<(), String> {
    std::process::Command::new(binary)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}
