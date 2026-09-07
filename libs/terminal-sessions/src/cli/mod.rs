mod activity;
mod builtins;
mod evidence;
mod interpreter;
mod model;
mod screen;
mod subscription;
mod tail;

use std::sync::Arc;

use crate::SessionFacts;

pub use builtins::{
    claude_tool, codex_tool, generic_tool, kimi_tool, pi_tool, session_file_containing_marker,
    session_file_for_external_id, CLAUDE_TOOL_ACCENT, CLAUDE_TOOL_ID, CODEX_TOOL_ACCENT,
    CODEX_TOOL_ID, GENERIC_TOOL_ACCENT, GENERIC_TOOL_ID, KIMI_TOOL_ACCENT, KIMI_TOOL_ID,
    PI_TOOL_ACCENT, PI_TOOL_ID,
};
pub use evidence::{
    CliActivityEvidence, CliLaunchProgram, CliModelCatalog, CliRuntimeState, CliScreenEvidence,
    CliSessionEvidence, CliViewportState,
};
pub use interpreter::{CliInterpreterError, CliSessionInterpreter};
pub use model::{CliSessionDescriptor, CliTool, CliToolColor, CliToolId};
pub use screen::{activity_signature, editor_draft, provider_error_line};
pub use subscription::{CliSessionSubscription, CliSessionSubscriptionError};

pub type CliSessionChangeHandler = Arc<dyn Fn() + Send + Sync + 'static>;

pub trait CliSessionStrategy: Send + Sync {
    fn tool(&self) -> &CliTool;

    fn priority(&self) -> i32 {
        0
    }

    fn matches(&self, session: &SessionFacts) -> bool;

    fn describe(&self, session: &SessionFacts) -> CliSessionDescriptor;

    fn transcript_completion(&self, _session: &SessionFacts, _marker: &str) -> Option<bool> {
        None
    }

    fn transcript_completion_at(
        &self,
        _paths: &[std::path::PathBuf],
        _marker: &str,
    ) -> Option<bool> {
        None
    }

    fn transcript_supported(&self) -> bool {
        false
    }

    fn transcript_paths(&self, _session: &SessionFacts) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    fn marked_report(&self, _paths: &[std::path::PathBuf], _marker: &str) -> Option<String> {
        None
    }

    fn transcript_report(
        &self,
        _paths: &[std::path::PathBuf],
        _since: std::time::SystemTime,
        _marker: &str,
    ) -> Option<String> {
        None
    }

    fn transcript_fault(
        &self,
        _paths: &[std::path::PathBuf],
        _since: std::time::SystemTime,
        _marker: &str,
    ) -> Option<String> {
        None
    }

    fn transcript_runtime(
        &self,
        _paths: &[std::path::PathBuf],
        _marker: &str,
    ) -> Option<CliRuntimeState> {
        None
    }

    fn interrupt_key(&self) -> &'static str {
        "esc"
    }

    fn subscribe(
        &self,
        _session: &SessionFacts,
        _on_change: CliSessionChangeHandler,
    ) -> Result<Option<CliSessionSubscription>, CliSessionSubscriptionError> {
        Ok(None)
    }

    fn subscription_dir(&self, _session: &SessionFacts) -> Option<std::path::PathBuf> {
        None
    }

    fn classify_screen(&self, _session: &SessionFacts, _screen: &str) -> CliScreenEvidence {
        CliScreenEvidence::default()
    }

    fn ui_rendered(&self, _screen: &str) -> bool {
        false
    }

    fn launch(&self) -> Option<CliLaunchProgram> {
        None
    }

    fn model_catalog(&self) -> Option<CliModelCatalog> {
        None
    }

    fn resume_args(&self, _external_id: &str) -> Option<Vec<String>> {
        None
    }
}
