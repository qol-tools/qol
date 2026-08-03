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

use self::environment::{PiEnvironment, SystemPiEnvironment};
use self::metadata::PiMetadataResolver;
use super::{pi_tool, project_name};

pub(super) struct PiStrategy {
    tool: CliTool,
    metadata: PiMetadataResolver,
}

impl Default for PiStrategy {
    fn default() -> Self {
        Self::with_environment(Arc::new(SystemPiEnvironment))
    }
}

impl PiStrategy {
    fn with_environment(environment: Arc<dyn PiEnvironment>) -> Self {
        Self {
            tool: pi_tool(),
            metadata: PiMetadataResolver::new(environment),
        }
    }
}

impl CliSessionStrategy for PiStrategy {
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
            .any(|process| process == "pi")
    }

    fn describe(&self, session: &SessionFacts) -> CliSessionDescriptor {
        let metadata = self.metadata.resolve(session);
        CliSessionDescriptor {
            tool: self.tool.clone(),
            display_name: metadata
                .session_name
                .or_else(|| title_session_name(&session.title, &session.cwd))
                .or_else(|| project_name(&session.cwd)),
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

fn title_session_name(title: &str, cwd: &str) -> Option<String> {
    let rest = title.trim().strip_prefix('\u{03C0}')?.trim_start();
    let rest = rest.strip_prefix('-')?.trim_start();
    let basename = project_name(cwd)?;
    let name = rest
        .strip_suffix(&basename)?
        .trim_end()
        .trim_end_matches('-')
        .trim_end();
    (!name.is_empty()).then(|| name.to_owned())
}
