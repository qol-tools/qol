use super::ExecutionPlatform;
use std::path::{Path, PathBuf};

pub(super) struct Platform;

impl ExecutionPlatform for Platform {
    fn resolve_candidate(
        _plugin_dir: &Path,
        _command_path: &Path,
        _canonical_plugin_dir: &Path,
    ) -> Option<PathBuf> {
        None
    }
}
