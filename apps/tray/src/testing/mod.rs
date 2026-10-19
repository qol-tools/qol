use std::sync::OnceLock;
use tokio::sync::Mutex;

#[cfg(feature = "dev")]
mod git_repo;
mod log_capture;

#[cfg(feature = "dev")]
pub(crate) use git_repo::GitRepo;
pub(crate) use log_capture::capture_logs;

pub(crate) fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn runtime_cache_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
