use std::path::PathBuf;

pub(super) fn log_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join("Library/Logs/qol-tray"))
        .unwrap_or_else(|| PathBuf::from("/tmp/qol-tray/logs"))
}
