use std::sync::OnceLock;
use tokio::sync::Mutex;

#[cfg(feature = "dev")]
mod git_repo;

#[cfg(feature = "dev")]
pub(crate) use git_repo::GitRepo;

pub(crate) fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn runtime_cache_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
