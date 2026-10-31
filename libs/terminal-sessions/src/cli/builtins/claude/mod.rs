mod environment;
mod history;
mod metadata;
mod search;
mod trust;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::cli::{
    ChatTurn, CliComposer, CliConversation, CliConversationMatch, CliDraftStash, CliLaunchProgram,
    CliRemoteControlToggle, CliResumeSettings, CliRuntimeState, CliScreenEvidence,
    CliSessionChangeHandler, CliSessionDescriptor, CliSessionEvidence, CliSessionStrategy,
    CliSessionSubscription, CliSessionSubscriptionError, CliTool, CliViewportState,
};
use crate::SessionFacts;

use self::environment::{ClaudeEnvironment, SystemClaudeEnvironment};
use self::history::ClaudeHistory;
use self::metadata::ClaudeMetadataResolver;
use super::{claude_tool, fallback_name};

pub(super) struct ClaudeStrategy {
    tool: CliTool,
    metadata: ClaudeMetadataResolver,
    history: ClaudeHistory,
}

impl Default for ClaudeStrategy {
    fn default() -> Self {
        Self::with_environment(Arc::new(SystemClaudeEnvironment::default()))
    }
}

impl ClaudeStrategy {
    fn with_environment(environment: Arc<dyn ClaudeEnvironment>) -> Self {
        Self {
            tool: claude_tool(),
            metadata: ClaudeMetadataResolver::new(environment),
            history: ClaudeHistory::system(),
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
                .or_else(|| fallback_name(session)),
            external_id: metadata.external_id,
            external_id_authoritative: false,
            has_activity: metadata.has_activity,
            evidence: CliSessionEvidence {
                runtime: metadata.runtime,
                activity: metadata.activity,
            },
        }
    }

    fn chat_transcript(&self, session: &SessionFacts) -> Option<Vec<ChatTurn>> {
        let path = self.metadata.subscription_path(session)?;
        metadata::chat_transcript(&path)
    }

    fn current_model(&self, session: &SessionFacts) -> Option<String> {
        let path = self.metadata.subscription_path(session)?;
        metadata::current_model(&path)
    }

    fn classify_screen(&self, _session: &SessionFacts, screen: &str) -> CliScreenEvidence {
        if crate::cli::screen::claude_working(screen) {
            CliScreenEvidence {
                viewport: CliViewportState::Live,
                runtime: CliRuntimeState::Working,
            }
        } else if crate::cli::screen::claude_dialog(screen) {
            CliScreenEvidence {
                viewport: CliViewportState::Live,
                runtime: CliRuntimeState::NeedsInput,
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

    fn ui_rendered(&self, screen: &str) -> bool {
        crate::cli::screen::claude_working(screen) || crate::cli::screen::has_done_marker(screen)
    }

    fn composer(&self, screen: &str) -> Option<CliComposer> {
        Some(crate::cli::composer::claude(screen))
    }

    fn launch(&self) -> Option<CliLaunchProgram> {
        Some(CliLaunchProgram::new("claude"))
    }

    fn launch_blocker(
        &self,
        cwd: &std::path::Path,
        config_dir: Option<&std::path::Path>,
    ) -> Option<String> {
        let user_home = dirs::home_dir()?;
        let config_file = config_dir.unwrap_or(&user_home).join(".claude.json");
        trust::untrusted_folder(&config_file, cwd, &user_home)
    }

    fn resume_args(&self, external_id: &str) -> Option<Vec<String>> {
        Some(vec!["--resume".to_owned(), external_id.to_owned()])
    }

    fn resume_settings(&self, session: &SessionFacts) -> CliResumeSettings {
        self.metadata.resume_settings(session)
    }

    fn closed_conversations(
        &self,
        homes: &[qol_agent_homes::AgentHome],
        since: std::time::SystemTime,
    ) -> Vec<CliConversation> {
        self.history
            .closed_conversations(&self.tool.id, homes, since)
    }

    fn search_conversations(
        &self,
        homes: &[qol_agent_homes::AgentHome],
        since: std::time::SystemTime,
        external_ids: &std::collections::HashSet<String>,
        terms: &[String],
    ) -> Vec<CliConversationMatch> {
        self.history
            .search_conversations(homes, since, external_ids, terms)
    }

    fn keep_open_reason(&self, session: &SessionFacts) -> Option<String> {
        self.metadata
            .remote_control_session(session)
            .map(|_| REMOTE_CONTROL_REASON.to_owned())
    }

    fn remote_control_url(&self, session: &SessionFacts) -> Option<String> {
        self.metadata
            .remote_control_session(session)
            .map(|id| format!("{REMOTE_CONTROL_URL}{id}"))
    }

    fn remote_control_toggle(&self) -> Option<CliRemoteControlToggle> {
        Some(CliRemoteControlToggle {
            command: "/remote-control",
            disconnect_choice: "Disconnect this session",
            menu_cursor: "\u{276F}",
            menu_up_key: "up",
            menu_select_key: "enter",
            menu_cancel_key: "esc",
        })
    }

    fn harness_busy(&self, session: &SessionFacts) -> bool {
        self.metadata.busy(session)
    }

    fn draft_stash(&self) -> Option<CliDraftStash> {
        Some(CliDraftStash {
            key: "ctrl+s",
            indicator: "\u{203A} stashed",
        })
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

const PLACEHOLDER_TITLE: &str = "Claude Code";
const REMOTE_CONTROL_REASON: &str = "Claude Code Remote Control is on";
const REMOTE_CONTROL_URL: &str = "https://claude.ai/code/";

fn clean_title(title: &str) -> Option<String> {
    let stripped = title.trim().trim_start_matches(|character: char| {
        let codepoint = character as u32;
        (0x2800..=0x28FF).contains(&codepoint)
            || (0x2733..=0x273F).contains(&codepoint)
            || character.is_whitespace()
    });
    let title = stripped.trim();
    (!title.is_empty() && title != PLACEHOLDER_TITLE).then(|| title.to_owned())
}
