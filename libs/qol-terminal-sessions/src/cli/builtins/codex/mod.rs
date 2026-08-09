mod environment;
mod metadata;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::cli::{
    CliLaunchProgram, CliRuntimeState, CliScreenEvidence, CliSessionChangeHandler,
    CliSessionDescriptor, CliSessionEvidence, CliSessionStrategy, CliSessionSubscription,
    CliSessionSubscriptionError, CliTool, CliViewportState,
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
            evidence: CliSessionEvidence {
                runtime: metadata.runtime,
                activity: metadata.activity,
            },
        }
    }

    fn classify_screen(&self, _session: &SessionFacts, screen: &str) -> CliScreenEvidence {
        let working = crate::cli::screen::has_interrupt_hint(screen);
        let awaiting = crate::cli::screen::has_numbered_choice(screen);
        let banner = crate::cli::screen::contains_any(
            screen,
            &["OpenAI Codex (v", "Tip: Try the Codex App"],
        );
        if working {
            CliScreenEvidence {
                viewport: CliViewportState::Live,
                runtime: CliRuntimeState::Working,
            }
        } else if awaiting {
            CliScreenEvidence {
                viewport: CliViewportState::Live,
                runtime: CliRuntimeState::NeedsInput,
            }
        } else if banner {
            CliScreenEvidence {
                viewport: CliViewportState::Historical,
                runtime: CliRuntimeState::Unknown,
            }
        } else {
            CliScreenEvidence::default()
        }
    }

    fn launch(&self) -> Option<CliLaunchProgram> {
        Some(CliLaunchProgram::new("codex"))
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
