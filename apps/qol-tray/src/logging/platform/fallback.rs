use std::path::PathBuf;

pub(crate) fn log_dir() -> PathBuf {
    std::env::temp_dir().join("qol-tray").join("logs")
}
