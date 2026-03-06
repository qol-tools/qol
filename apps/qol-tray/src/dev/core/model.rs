use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildStatus {
    Queued,
    Building,
    Skipped,
    Success,
    Failed,
}

impl BuildStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Building => "building",
            Self::Skipped => "skipped",
            Self::Success => "success",
            Self::Failed => "failed",
        }
    }
}

impl Default for BuildStatus {
    fn default() -> Self {
        Self::Queued
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreBuildResult {
    pub plugin_id: String,
    pub success: bool,
    pub output: String,
    pub skipped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreInput {
    RunStarted,
    PluginProgress {
        plugin_id: String,
        status: BuildStatus,
        percent: u8,
        phase: String,
    },
    RunFinished {
        results: Vec<CoreBuildResult>,
    },
}

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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoreBuildProgress {
    pub status: BuildStatus,
    pub percent: u8,
    pub phase: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoreState {
    pub building: bool,
    pub progress: HashMap<String, CoreBuildProgress>,
}
