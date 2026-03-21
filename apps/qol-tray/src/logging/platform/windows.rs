use std::path::PathBuf;

pub(super) fn log_dir() -> PathBuf {
    dirs::data_local_dir()
        .map(|d| d.join("qol-tray/logs"))
        .unwrap_or_else(|| PathBuf::from("C:/Temp/qol-tray/logs"))
}
