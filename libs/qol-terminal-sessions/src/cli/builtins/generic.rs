use crate::cli::{
    CliRuntimeState, CliScreenEvidence, CliSessionDescriptor, CliSessionEvidence,
    CliSessionStrategy, CliTool, CliViewportState,
};
use crate::SessionFacts;

use super::{generic_tool, project_name};

pub(in crate::cli) struct GenericStrategy {
    tool: CliTool,
}

impl Default for GenericStrategy {
    fn default() -> Self {
        Self {
            tool: generic_tool(),
        }
    }
}

impl CliSessionStrategy for GenericStrategy {
    fn tool(&self) -> &CliTool {
        &self.tool
    }

    fn matches(&self, _session: &SessionFacts) -> bool {
        true
    }

    fn interrupt_key(&self) -> &'static str {
        "ctrl+c"
    }

    fn classify_screen(&self, session: &SessionFacts, _screen: &str) -> CliScreenEvidence {
        if session.at_prompt {
            CliScreenEvidence {
                viewport: CliViewportState::Unknown,
                runtime: CliRuntimeState::Ready,
            }
        } else {
            CliScreenEvidence::default()
        }
    }

    fn describe(&self, session: &SessionFacts) -> CliSessionDescriptor {
        let reported = session.reported_cmd.as_deref().map(str::trim);
        let title = session.title.trim();
        let display_name = reported
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| (!title.is_empty()).then(|| title.to_owned()))
            .or_else(|| project_name(&session.cwd));
        CliSessionDescriptor {
            tool: self.tool.clone(),
            display_name,
            external_id: None,
            has_activity: None,
            evidence: CliSessionEvidence::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::{CliRuntimeState, CliScreenEvidence, CliSessionStrategy, CliViewportState};
    use crate::{BackendId, SessionCapabilities, SessionFacts, SessionId};

    use super::GenericStrategy;

    #[test]
    fn generic_has_no_launch_program_and_no_strong_evidence() {
        let strategy = GenericStrategy::default();
        assert_eq!(strategy.launch(), None);
        let descriptor = strategy.describe(&session(false));
        assert_eq!(descriptor.evidence.runtime, CliRuntimeState::Unknown);
        assert_eq!(descriptor.evidence.activity, Default::default());
    }

    #[test]
    fn generic_classifies_a_prompted_shell_as_live_and_ready() {
        let strategy = GenericStrategy::default();
        assert_eq!(
            strategy.classify_screen(&session(true), "kmrh47@host:~$"),
            CliScreenEvidence {
                viewport: CliViewportState::Unknown,
                runtime: CliRuntimeState::Ready,
            }
        );
        assert_eq!(
            strategy.classify_screen(&session(false), "building..."),
            CliScreenEvidence::default()
        );
    }

    fn session(at_prompt: bool) -> SessionFacts {
        SessionFacts {
            id: SessionId::new(BackendId::new("test").unwrap(), "1").unwrap(),
            root_pid: 1,
            cwd: "/work/project".to_owned(),
            title: "Terminal".to_owned(),
            at_prompt,
            reported_cmd: None,
            foreground_basenames: vec!["bash".to_owned()],
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: None,
        }
    }
}
