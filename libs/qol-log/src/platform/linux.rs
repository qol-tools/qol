use std::path::PathBuf;

pub(crate) fn log_dir() -> PathBuf {
    qol_config::data_dir()
        .map(|p| p.join("logs"))
        .unwrap_or_else(|| std::env::temp_dir().join("qol-tray/logs"))
}
