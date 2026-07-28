use std::path::PathBuf;

pub(super) fn model_cache_dir() -> Option<PathBuf> {
    Some(hf_hub::Cache::from_env().path().clone())
}
