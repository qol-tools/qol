use super::types::{BuildStatus, CoreBuildResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEvent {
    BuildStarted,
    BuildPluginProgress {
        plugin_id: String,
        status: BuildStatus,
        percent: u8,
        phase: String,
    },
    BuildComplete {
        results: Vec<CoreBuildResult>,
    },
}
