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
    claude_tool, codex_tool, generic_tool, kimi_tool, pi_tool, CLAUDE_TOOL_ACCENT, CLAUDE_TOOL_ID,
    CODEX_TOOL_ACCENT, CODEX_TOOL_ID, GENERIC_TOOL_ACCENT, GENERIC_TOOL_ID, KIMI_TOOL_ACCENT,
    KIMI_TOOL_ID, PI_TOOL_ACCENT, PI_TOOL_ID,
};
pub use evidence::{
    CliActivityEvidence, CliLaunchProgram, CliModelCatalog, CliRuntimeState, CliScreenEvidence,
    CliSessionEvidence, CliViewportState,
};
pub use interpreter::{CliInterpreterError, CliSessionInterpreter};
pub use model::{CliSessionDescriptor, CliTool, CliToolColor, CliToolId};
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
