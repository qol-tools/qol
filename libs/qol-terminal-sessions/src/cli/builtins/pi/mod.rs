mod environment;
pub use self::environment::{session_file_containing_marker, session_file_for_external_id};
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
            external_id_authoritative: metadata.external_id_authoritative,
            has_activity: metadata.has_activity,
            evidence: CliSessionEvidence {
                runtime: metadata.runtime,
                activity: metadata.activity,
            },
        }
    }

    fn transcript_completion(&self, session: &SessionFacts, marker: &str) -> Option<bool> {
        let paths = self.metadata.subscription_paths(session);
        self.transcript_completion_at(&paths, marker)
    }

    fn transcript_completion_at(&self, paths: &[std::path::PathBuf], marker: &str) -> Option<bool> {
        for path in paths {
            match metadata::marker_in_terminal_assistant_text(path, marker) {
                Some(true) => return Some(true),
                Some(false) if paths.len() == 1 => return Some(false),
                _ => {}
            }
        }
        None
    }

    fn transcript_supported(&self) -> bool {
        true
    }

    fn transcript_paths(&self, session: &SessionFacts) -> Vec<std::path::PathBuf> {
        self.metadata.subscription_paths(session)
    }

    fn marked_report(&self, paths: &[std::path::PathBuf], marker: &str) -> Option<String> {
        for path in paths {
            if let Some(text) = metadata::marked_terminal_text(path, marker) {
                return Some(text);
            }
        }
        None
    }

    fn transcript_report(
        &self,
        paths: &[std::path::PathBuf],
        since: std::time::SystemTime,
        marker: &str,
    ) -> Option<String> {
        let since_millis = since
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|elapsed| elapsed.as_millis() as i64)?;
        metadata::transcript_report(paths, since_millis, marker)
    }

    fn transcript_fault(
        &self,
        paths: &[std::path::PathBuf],
        since: std::time::SystemTime,
        marker: &str,
    ) -> Option<String> {
        let since_millis = since
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|elapsed| elapsed.as_millis() as i64)?;
        metadata::transcript_fault(paths, since_millis, marker)
    }

    fn transcript_runtime(
        &self,
        paths: &[std::path::PathBuf],
        marker: &str,
    ) -> Option<CliRuntimeState> {
        metadata::transcript_runtime(paths, marker)
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

    fn subscription_dir(&self, session: &SessionFacts) -> Option<std::path::PathBuf> {
        self.metadata.subscription_dir(session)
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
