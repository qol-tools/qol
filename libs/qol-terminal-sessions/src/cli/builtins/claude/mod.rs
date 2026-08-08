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

use self::environment::{ClaudeEnvironment, SystemClaudeEnvironment};
use self::metadata::ClaudeMetadataResolver;
use super::{claude_tool, project_name};

pub(super) struct ClaudeStrategy {
    tool: CliTool,
    metadata: ClaudeMetadataResolver,
}

impl Default for ClaudeStrategy {
    fn default() -> Self {
        Self::with_environment(Arc::new(SystemClaudeEnvironment))
    }
}

impl ClaudeStrategy {
    fn with_environment(environment: Arc<dyn ClaudeEnvironment>) -> Self {
        Self {
            tool: claude_tool(),
            metadata: ClaudeMetadataResolver::new(environment),
        }
    }
}

impl CliSessionStrategy for ClaudeStrategy {
    fn tool(&self) -> &CliTool {
        &self.tool
    }

    fn priority(&self) -> i32 {
        100
    }

    fn matches(&self, session: &SessionFacts) -> bool {
        session
            .foreground_basenames
            .iter()
            .any(|process| process == "claude")
    }

    fn describe(&self, session: &SessionFacts) -> CliSessionDescriptor {
        let metadata = self.metadata.resolve(session);
        CliSessionDescriptor {
            tool: self.tool.clone(),
            display_name: metadata
                .custom_title
                .or_else(|| clean_title(&session.title))
                .or_else(|| project_name(&session.cwd)),
            external_id: metadata.external_id,
            has_activity: metadata.has_activity,
            evidence: CliSessionEvidence {
                runtime: CliRuntimeState::Unknown,
                activity: metadata.activity,
            },
        }
    }

    fn classify_screen(&self, _session: &SessionFacts, screen: &str) -> CliScreenEvidence {
        if crate::cli::screen::has_interrupt_hint(screen)
            || crate::cli::screen::has_live_spinner(screen)
        {
            CliScreenEvidence {
                viewport: CliViewportState::Live,
                runtime: CliRuntimeState::Working,
            }
        } else if crate::cli::screen::has_done_marker(screen) {
            CliScreenEvidence {
                viewport: CliViewportState::Unknown,
                runtime: CliRuntimeState::Ready,
            }
        } else {
            CliScreenEvidence::default()
        }
    }

    fn launch(&self) -> Option<CliLaunchProgram> {
        Some(CliLaunchProgram::new("claude"))
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

fn clean_title(title: &str) -> Option<String> {
    let stripped = title.trim().trim_start_matches(|character: char| {
        let codepoint = character as u32;
        (0x2800..=0x28FF).contains(&codepoint)
            || (0x2733..=0x273F).contains(&codepoint)
            || character.is_whitespace()
    });
    let title = stripped.trim();
    (!title.is_empty()).then(|| title.to_owned())
}
