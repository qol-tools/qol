use std::collections::HashMap;
use std::path::PathBuf;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use fallback as active;
#[cfg(target_os = "linux")]
use linux as active;
#[cfg(target_os = "macos")]
use macos as active;

pub(super) fn default_cache_dir(env: &HashMap<String, String>) -> Option<PathBuf> {
    active::default_cache_dir(env)
}
