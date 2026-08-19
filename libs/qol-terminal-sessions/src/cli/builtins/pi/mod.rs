mod environment;
mod metadata;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::cli::{
    CliLaunchProgram, CliModelCatalog, CliRuntimeState, CliScreenEvidence, CliSessionChangeHandler,
    CliSessionDescriptor, CliSessionEvidence, CliSessionStrategy, CliSessionSubscription,
    CliSessionSubscriptionError, CliTool, CliViewportState,
};
use crate::SessionFacts;

use self::environment::{PiEnvironment, SystemPiEnvironment};
use self::metadata::PiMetadataResolver;
use super::{fallback_name, pi_tool, project_name};

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
            display_name: title_session_name(&session.title, &session.cwd)
                .or(metadata.session_name)
                .or_else(|| fallback_name(session)),
            external_id: metadata.external_id,
            has_activity: metadata.has_activity,
            evidence: CliSessionEvidence {
                runtime: metadata.runtime,
                activity: metadata.activity,
            },
        }
    }

    fn transcript_completion(&self, session: &SessionFacts, marker: &str) -> Option<bool> {
        let path = self.metadata.subscription_path(session)?;
        metadata::marker_in_terminal_assistant_text(&path, marker)
    }

    fn classify_screen(&self, _session: &SessionFacts, screen: &str) -> CliScreenEvidence {
        if !crate::cli::screen::pi_live(screen) {
            return CliScreenEvidence::default();
        }
        if crate::cli::screen::has_braille_spinner(screen) {
            CliScreenEvidence {
                viewport: CliViewportState::Live,
                runtime: CliRuntimeState::Working,
            }
        } else if crate::cli::screen::has_choice_hint(screen)
            || crate::cli::screen::has_picker_cluster(screen)
        {
            CliScreenEvidence {
                viewport: CliViewportState::Live,
                runtime: CliRuntimeState::NeedsInput,
            }
        } else if crate::cli::screen::contains_any(screen, &["to show full startup help"]) {
            CliScreenEvidence {
                viewport: CliViewportState::Historical,
                runtime: CliRuntimeState::Unknown,
            }
        } else {
            CliScreenEvidence::default()
        }
    }

    fn ui_rendered(&self, screen: &str) -> bool {
        crate::cli::screen::pi_live(screen)
    }

    fn launch(&self) -> Option<CliLaunchProgram> {
        Some(CliLaunchProgram::new("pi"))
    }

    fn resume_args(&self, external_id: &str) -> Option<Vec<String>> {
        Some(vec!["--session".to_owned(), external_id.to_owned()])
    }

    fn model_catalog(&self) -> Option<CliModelCatalog> {
        Some(
            CliModelCatalog::new("pi", ["--list-models"])
                .model_column(1)
                .header_rows(1),
        )
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
