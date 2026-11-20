use std::collections::HashMap;
use std::path::PathBuf;

pub(super) fn default_cache_dir(_env: &HashMap<String, String>) -> Option<PathBuf> {
    None
}
