mod environment;
mod metadata;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::cli::{
    CliSessionChangeHandler, CliSessionDescriptor, CliSessionStrategy, CliSessionSubscription,
    CliSessionSubscriptionError, CliTool,
};
use crate::SessionFacts;

use self::environment::{CodexEnvironment, SystemCodexEnvironment};
use self::metadata::CodexMetadataResolver;
use super::{codex_tool, project_name};

pub(super) struct CodexStrategy {
    tool: CliTool,
    metadata: CodexMetadataResolver,
}

impl Default for CodexStrategy {
    fn default() -> Self {
        Self::with_environment(Arc::new(SystemCodexEnvironment))
    }
}

impl CodexStrategy {
    fn with_environment(environment: Arc<dyn CodexEnvironment>) -> Self {
        Self {
            tool: codex_tool(),
            metadata: CodexMetadataResolver::new(environment),
        }
    }
}

impl CliSessionStrategy for CodexStrategy {
    fn tool(&self) -> &CliTool {
        &self.tool
    }

    fn priority(&self) -> i32 {
        110
    }

    fn matches(&self, session: &SessionFacts) -> bool {
        session
            .foreground_basenames
            .iter()
            .any(|process| process == "codex")
    }

    fn describe(&self, session: &SessionFacts) -> CliSessionDescriptor {
        let metadata = self.metadata.resolve(session);
        CliSessionDescriptor {
            tool: self.tool.clone(),
            display_name: metadata.thread_name.or_else(|| project_name(&session.cwd)),
            external_id: metadata.external_id,
            has_activity: metadata.has_activity,
        }
    }

    fn subscribe(
        &self,
        _session: &SessionFacts,
        on_change: CliSessionChangeHandler,
    ) -> Result<Option<CliSessionSubscription>, CliSessionSubscriptionError> {
        self.metadata
            .subscription_path()
            .map(|path| CliSessionSubscription::watch_file(path, on_change))
            .transpose()
    }
}
