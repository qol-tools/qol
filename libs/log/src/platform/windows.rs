use std::path::PathBuf;

pub(crate) fn log_dir() -> PathBuf {
    qol_config::data_dir()
        .map(|dir| dir.join("logs"))
        .unwrap_or_else(|| PathBuf::from("C:/Temp/qol-tray/logs"))
}
