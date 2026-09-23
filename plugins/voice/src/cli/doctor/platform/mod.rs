use std::path::PathBuf;

#[cfg(not(target_os = "linux"))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(target_os = "linux"))]
use fallback as selected;
#[cfg(target_os = "linux")]
use linux as selected;

pub(super) fn model_cache_dir() -> Option<PathBuf> {
    selected::model_cache_dir()
}
