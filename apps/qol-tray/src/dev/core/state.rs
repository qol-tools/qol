use std::collections::HashMap;

use super::types::BuildStatus;

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
