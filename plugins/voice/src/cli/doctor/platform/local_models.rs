use std::path::PathBuf;

#[cfg(feature = "local-stt")]
pub(super) fn model_cache_dir() -> Option<PathBuf> {
    Some(hf_hub::Cache::from_env().path().clone())
}

#[cfg(not(feature = "local-stt"))]
pub(super) fn model_cache_dir() -> Option<PathBuf> {
    None
}
