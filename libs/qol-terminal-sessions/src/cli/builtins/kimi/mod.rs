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

use self::environment::{KimiEnvironment, SystemKimiEnvironment};
use self::metadata::KimiMetadataResolver;
use super::{kimi_tool, project_name};

pub(super) struct KimiStrategy {
    tool: CliTool,
    metadata: KimiMetadataResolver,
}

impl Default for KimiStrategy {
    fn default() -> Self {
        Self::with_environment(Arc::new(SystemKimiEnvironment))
    }
}

impl KimiStrategy {
    fn with_environment(environment: Arc<dyn KimiEnvironment>) -> Self {
        Self {
            tool: kimi_tool(),
            metadata: KimiMetadataResolver::new(environment),
        }
    }
}

impl CliSessionStrategy for KimiStrategy {
    fn tool(&self) -> &CliTool {
        &self.tool
    }

    fn priority(&self) -> i32 {
        90
    }

    fn matches(&self, session: &SessionFacts) -> bool {
        session
            .foreground_basenames
            .iter()
            .any(|process| matches!(process.as_str(), "kimi" | "kimi-co" | "kimi-code"))
    }

    fn describe(&self, session: &SessionFacts) -> CliSessionDescriptor {
        let metadata = self.metadata.resolve(session);
        CliSessionDescriptor {
            tool: self.tool.clone(),
            display_name: metadata.session_name.or_else(|| project_name(&session.cwd)),
            external_id: metadata.external_id,
            has_activity: metadata.has_activity,
        }
    }

    fn subscribe(
        &self,
        session: &SessionFacts,
        on_change: CliSessionChangeHandler,
    ) -> Result<Option<CliSessionSubscription>, CliSessionSubscriptionError> {
        self.metadata
            .subscription_path(session)
            .map(|path| CliSessionSubscription::watch_file(path, on_change))
            .transpose()
    }
}
