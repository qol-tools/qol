use serde::Serialize;
use std::path::PathBuf;

use crate::plugins::daemon_tracker::ManagedProcess;

pub(super) const REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ArtifactId {
    StopProcesses,
    ShellHookBash,
    ShellHookZsh,
    Autostart,
    DesktopEntry,
    MimeDefault,
    MimeData,
    MimeCache,
    Icon64,
    Icon128,
    Icon256,
    RuntimeDirectory,
    ModeConfig,
    ActiveInstallId,
    StagedBinary,
    Binary,
    InstallMarker,
    RefreshDesktopCaches,
    ConfigDirectory,
    DataDirectory,
}

impl ArtifactId {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::StopProcesses => "Owned processes",
            Self::ShellHookBash => "Bash shell hook",
            Self::ShellHookZsh => "Zsh shell hook",
            Self::Autostart => "Autostart entry",
            Self::DesktopEntry => "Desktop entry",
            Self::MimeDefault => "MIME default",
            Self::MimeData => "MIME data association",
            Self::MimeCache => "MIME cache association",
            Self::Icon64 => "64px icon",
            Self::Icon128 => "128px icon",
            Self::Icon256 => "256px icon",
            Self::RuntimeDirectory => "Runtime directory",
            Self::ModeConfig => "Runtime mode",
            Self::ActiveInstallId => "Active install marker",
            Self::StagedBinary => "Staged binary",
            Self::Binary => "Installed binary",
            Self::InstallMarker => "Binary ownership marker",
            Self::RefreshDesktopCaches => "Desktop caches",
            Self::ConfigDirectory => "Configuration and profile data",
            Self::DataDirectory => "Application data",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    StopProcesses,
    RemoveFile,
    RemoveDirectory,
    EditShellHook,
    EditMimeAssociation,
    RefreshDesktopCaches,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TargetState {
    Present,
    Absent,
    Unowned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ActionResult {
    Planned,
    Removed,
    Updated,
    Stopped,
    AlreadyAbsent,
    SkippedUnowned,
    SkippedDependency,
    Failed,
}

impl ActionResult {
    pub(super) fn is_incomplete(self) -> bool {
        matches!(
            self,
            Self::SkippedUnowned | Self::SkippedDependency | Self::Failed
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReportStatus {
    Planned,
    Complete,
    Partial,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ActionReport {
    pub(super) id: ArtifactId,
    pub(super) operation: Operation,
    pub(super) target: String,
    pub(super) state: TargetState,
    pub(super) result: ActionResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct PreservedReport {
    pub(super) id: ArtifactId,
    pub(super) path: PathBuf,
    pub(super) state: TargetState,
    pub(super) reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct UninstallReport {
    pub(super) schema_version: u32,
    pub(super) platform: String,
    pub(super) dry_run: bool,
    pub(super) purge_data: bool,
    pub(super) status: ReportStatus,
    pub(super) actions: Vec<ActionReport>,
    pub(super) preserved: Vec<PreservedReport>,
    pub(super) warnings: Vec<String>,
}

impl UninstallReport {
    pub(super) fn is_partial(&self) -> bool {
        self.status == ReportStatus::Partial
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Options {
    pub(super) dry_run: bool,
    pub(super) json: bool,
    pub(super) purge_data: bool,
    pub(super) skip_shell_hook: bool,
}

#[derive(Clone, Debug)]
pub(super) enum OwnershipProof {
    AnyFile,
    AnyDirectory,
    BinaryWithMarker(PathBuf),
    ExactBytes(&'static [u8]),
    MimeAssociation,
    ShellHook,
    TextMarkers(&'static [&'static str]),
    ValidInstallId,
}

#[derive(Clone, Debug)]
pub(super) struct ArtifactSpec {
    pub(super) id: ArtifactId,
    pub(super) operation: Operation,
    pub(super) path: PathBuf,
    pub(super) ownership: OwnershipProof,
    pub(super) depends_on: Vec<ArtifactId>,
}

#[derive(Clone, Debug)]
pub(super) struct PreserveSpec {
    pub(super) id: ArtifactId,
    pub(super) path: PathBuf,
    pub(super) reason: &'static str,
}

#[derive(Clone, Debug)]
pub(super) struct UninstallContext {
    pub(super) platform: &'static str,
    pub(super) artifacts: Vec<ArtifactSpec>,
    pub(super) purge_artifacts: Vec<ArtifactSpec>,
    pub(super) preserved: Vec<PreserveSpec>,
    pub(super) refresh_root: PathBuf,
}

#[derive(Clone, Debug)]
pub(super) struct ProcessTargets {
    pub(super) installed_binary: Option<PathBuf>,
    pub(super) plugins: Vec<ManagedProcess>,
}

#[derive(Clone, Debug)]
pub(super) enum PlanTarget {
    Path(PathBuf),
    Processes(ProcessTargets),
}

impl PlanTarget {
    pub(super) fn display(&self) -> String {
        match self {
            Self::Path(path) => path.display().to_string(),
            Self::Processes(targets) => format!(
                "QoL Tray and {} owned plugin daemon(s)",
                targets.plugins.len()
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct PlanItem {
    pub(super) id: ArtifactId,
    pub(super) operation: Operation,
    pub(super) target: PlanTarget,
    pub(super) state: TargetState,
    pub(super) depends_on: Vec<ArtifactId>,
}

impl PlanItem {
    pub(super) fn report(&self, result: ActionResult, error: Option<String>) -> ActionReport {
        ActionReport {
            id: self.id,
            operation: self.operation,
            target: self.target.display(),
            state: self.state,
            result,
            error,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct UninstallPlan {
    pub(super) platform: &'static str,
    pub(super) purge_data: bool,
    pub(super) items: Vec<PlanItem>,
    pub(super) preserved: Vec<PreservedReport>,
    pub(super) warnings: Vec<String>,
}
