use std::path::PathBuf;

pub(super) fn log_dir() -> PathBuf {
    crate::paths::base_data_dir()
        .map(|p| p.join("logs"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/qol-tray/logs"))
}
