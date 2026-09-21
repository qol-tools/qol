use super::ExecutionPlatform;
use std::path::{Path, PathBuf};

pub(super) struct Platform;

impl ExecutionPlatform for Platform {
    fn resolve_candidate(
        plugin_dir: &Path,
        command_path: &Path,
        canonical_plugin_dir: &Path,
    ) -> Option<PathBuf> {
        let primary = plugin_dir.join(command_path.as_os_str());
        if primary.extension().is_some() {
            return None;
        }
        let candidate = primary.with_extension("exe");
        super::super::is_allowed_candidate(&candidate, canonical_plugin_dir).then_some(candidate)
    }
}
