use std::path::PathBuf;

pub(crate) fn log_dir() -> PathBuf {
    crate::paths::base_data_dir()
        .map(|p| p.join("logs"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/qol-tray/logs"))
}
