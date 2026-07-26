mod builtins;
mod interpreter;
mod model;
mod subscription;

use std::sync::Arc;

use crate::SessionFacts;

pub use builtins::{
    claude_tool, codex_tool, generic_tool, CLAUDE_TOOL_ACCENT, CLAUDE_TOOL_ID, CODEX_TOOL_ACCENT,
    CODEX_TOOL_ID, GENERIC_TOOL_ACCENT, GENERIC_TOOL_ID,
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

    fn subscribe(
        &self,
        _session: &SessionFacts,
        _on_change: CliSessionChangeHandler,
    ) -> Result<Option<CliSessionSubscription>, CliSessionSubscriptionError> {
        Ok(None)
    }
}
