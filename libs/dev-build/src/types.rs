use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::core::BuildStatus;

#[derive(Debug, Clone, Serialize)]
pub struct BuildResult {
    pub plugin_id: String,
    pub success: bool,
    pub output: String,
    pub skipped: bool,
    #[serde(skip_serializing)]
    pub artifacts: Vec<crate::cargo_build::CargoArtifact>,
}

#[derive(Debug, Clone)]
pub struct PluginBuildPlan {
    pub plugin_id: String,
    pub path: PathBuf,
    pub has_cargo: bool,
    pub supports_platform: bool,
    pub needs_rebuild: bool,
    pub current_fingerprint: Option<String>,
    pub last_built_fingerprint: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct BuildRun {
    pub plans: Vec<PluginBuildPlan>,
    pub results: Vec<BuildResult>,
    pub fingerprints: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct PluginBuildProgress {
    pub plugin_id: String,
    pub status: BuildStatus,
    pub percent: u8,
    pub phase: String,
}
