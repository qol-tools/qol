use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::SessionFacts;

use super::builtins::GenericStrategy;
use super::model::normalize_display_name;
use super::{
    CliLaunchProgram, CliModelCatalog, CliScreenEvidence, CliSessionChangeHandler,
    CliSessionDescriptor, CliSessionStrategy, CliSessionSubscription, CliSessionSubscriptionError,
    CliToolId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliInterpreterError {
    duplicate: CliToolId,
}

impl Display for CliInterpreterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "CLI session strategy `{}` was registered twice",
            self.duplicate
        )
    }
}

impl std::error::Error for CliInterpreterError {}

pub struct CliSessionInterpreter {
    strategies: Vec<Arc<dyn CliSessionStrategy>>,
    fallback: GenericStrategy,
}

impl CliSessionInterpreter {
    pub fn system() -> Self {
        Self::from_strategies(super::builtins::system_strategies())
            .expect("built-in CLI session strategy ids are unique")
    }

    pub fn from_strategies(
        strategies: impl IntoIterator<Item = Arc<dyn CliSessionStrategy>>,
    ) -> Result<Self, CliInterpreterError> {
        let mut registered = BTreeSet::new();
        let mut strategies = strategies.into_iter().collect::<Vec<_>>();
        for strategy in &strategies {
            let id = strategy.tool().id.clone();
            if !registered.insert(id.clone()) {
                return Err(CliInterpreterError { duplicate: id });
            }
        }
        strategies.sort_by(|left, right| {
            right
                .priority()
                .cmp(&left.priority())
                .then_with(|| left.tool().id.cmp(&right.tool().id))
        });
        Ok(Self {
            strategies,
            fallback: GenericStrategy::default(),
        })
    }

    pub fn describe(&self, session: &SessionFacts) -> CliSessionDescriptor {
        let strategy = self.strategy_for(session);
        let mut descriptor = strategy.describe(session);
        descriptor.display_name = normalize_display_name(descriptor.display_name);
        qol_runtime::probe!(
            "CLI_SESSION_INTERPRETATION",
            "event=described tool={} terminal_backend={}",
            descriptor.tool.id,
            session.id.backend()
        );
        descriptor
    }

    pub fn interrupt_key(&self, session: &SessionFacts) -> &'static str {
        self.strategy_for(session).interrupt_key()
    }

    pub fn classify_screen(&self, session: &SessionFacts, screen: &str) -> CliScreenEvidence {
        self.strategy_for(session).classify_screen(session, screen)
    }

    pub fn ui_rendered(&self, session: &SessionFacts, screen: &str) -> bool {
        self.strategy_for(session).ui_rendered(screen)
    }

    pub fn launch_for(&self, tool: &CliToolId) -> Option<CliLaunchProgram> {
        self.strategies
            .iter()
            .find(|strategy| &strategy.tool().id == tool)
            .and_then(|strategy| strategy.launch())
    }

    pub fn model_catalog_for(&self, tool: &CliToolId) -> Option<CliModelCatalog> {
        self.strategies
            .iter()
            .find(|strategy| &strategy.tool().id == tool)
            .and_then(|strategy| strategy.model_catalog())
    }

    pub fn resume_args_for(&self, tool: &CliToolId, external_id: &str) -> Option<Vec<String>> {
        self.strategies
            .iter()
            .find(|strategy| &strategy.tool().id == tool)
            .and_then(|strategy| strategy.resume_args(external_id))
    }

    pub fn launchable_tools(&self) -> Vec<CliToolId> {
        self.strategies
            .iter()
            .filter(|strategy| strategy.launch().is_some())
            .map(|strategy| strategy.tool().id.clone())
            .collect()
    }

    pub fn subscribe(
        &self,
        session: &SessionFacts,
        on_change: CliSessionChangeHandler,
    ) -> Result<Option<CliSessionSubscription>, CliSessionSubscriptionError> {
        let strategy = self.strategy_for(session);
        let subscription = strategy.subscribe(session, on_change)?;
        qol_runtime::probe!(
            "CLI_SESSION_INTERPRETATION",
            "event=subscription state={} tool={} terminal_backend={}",
            if subscription.is_some() {
                "active"
            } else {
                "unsupported"
            },
            strategy.tool().id,
            session.id.backend()
        );
        Ok(subscription)
    }

    fn strategy_for<'a>(&'a self, session: &SessionFacts) -> &'a dyn CliSessionStrategy {
        self.strategies
            .iter()
            .find(|strategy| strategy.matches(session))
            .map(Arc::as_ref)
            .unwrap_or(&self.fallback)
    }
}

impl Default for CliSessionInterpreter {
    fn default() -> Self {
        Self::system()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::cli::{
        generic_tool, CliActivityEvidence, CliRuntimeState, CliScreenEvidence,
        CliSessionDescriptor, CliSessionEvidence, CliSessionInterpreter, CliSessionStrategy,
        CliTool, CliToolColor, CliToolId, CliViewportState,
    };
    use crate::{BackendId, SessionCapabilities, SessionFacts, SessionId};

    struct NamedStrategy {
        tool: CliTool,
        process: &'static str,
        priority: i32,
    }

    impl CliSessionStrategy for NamedStrategy {
        fn tool(&self) -> &CliTool {
            &self.tool
        }

        fn priority(&self) -> i32 {
            self.priority
        }

        fn matches(&self, session: &SessionFacts) -> bool {
            session
                .foreground_basenames
                .iter()
                .any(|process| process == self.process)
        }

        fn describe(&self, _session: &SessionFacts) -> CliSessionDescriptor {
            CliSessionDescriptor {
                tool: self.tool.clone(),
                display_name: Some(self.tool.label.clone()),
                external_id: None,
                has_activity: None,
                evidence: CliSessionEvidence::default(),
            }
        }
    }

    #[test]
    fn registered_strategies_override_the_generic_fallback_by_priority() {
        let lower = strategy("lower", "Lower", "agent", 1);
        let higher = strategy("higher", "Higher", "agent", 2);
        let interpreter = CliSessionInterpreter::from_strategies([lower, higher]).unwrap();

        let specialized = interpreter.describe(&session(&["agent"]));
        let generic = interpreter.describe(&session(&["bash"]));

        assert_eq!(specialized.tool.id.as_str(), "higher");
        assert_eq!(generic.tool, generic_tool());
    }

    #[test]
    fn duplicate_strategy_ids_are_rejected() {
        let first = strategy("agent", "First", "one", 1);
        let second = strategy("agent", "Second", "two", 2);

        let error = CliSessionInterpreter::from_strategies([first, second])
            .err()
            .expect("duplicate strategy ids must fail");

        assert!(error.to_string().contains("registered twice"));
    }

    #[test]
    fn system_interpretation_specializes_known_tools_and_falls_back_for_everything_else() {
        let interpreter = CliSessionInterpreter::system();

        let codex = interpreter.describe(&session(&["zsh", "codex", "claude"]));
        let claude = interpreter.describe(&session(&["zsh", "claude"]));
        let arbitrary = interpreter.describe(&session(&["zsh", "my-future-cli"]));

        assert_eq!(codex.tool.id.as_str(), "codex");
        assert_eq!(claude.tool.id.as_str(), "claude");
        assert_eq!(arbitrary.tool, generic_tool());
    }

    #[test]
    fn interrupt_key_is_esc_for_agent_tools_and_ctrl_c_for_generic_shells() {
        let interpreter = CliSessionInterpreter::system();

        assert_eq!(
            interpreter.interrupt_key(&session(&["zsh", "claude"])),
            "esc"
        );
        assert_eq!(interpreter.interrupt_key(&session(&["bash"])), "ctrl+c");
    }

    #[test]
    fn builtins_expose_launch_programs_and_unknown_ids_return_none() {
        let interpreter = CliSessionInterpreter::system();

        let cases = [
            ("codex", "codex"),
            ("claude", "claude"),
            ("pi", "pi"),
            ("kimi", "kimi"),
        ];
        for (tool, program) in cases {
            let launch = interpreter
                .launch_for(&CliToolId::new(tool).unwrap())
                .expect("built-in tools must offer a launch program");
            assert_eq!(launch.program, program, "tool: {tool}");
            assert!(launch.args.is_empty(), "tool: {tool}");
        }
        assert_eq!(
            interpreter.launch_for(&CliToolId::new("generic").unwrap()),
            None
        );
        assert_eq!(
            interpreter.launch_for(&CliToolId::new("future-tool").unwrap()),
            None
        );
    }

    #[test]
    fn resume_args_map_known_tools_and_unknown_tools_have_no_resume_flag() {
        let interpreter = CliSessionInterpreter::system();

        let cases = [
            ("pi", vec!["--session-id".to_owned(), "abc".to_owned()]),
            ("codex", vec!["resume".to_owned(), "abc".to_owned()]),
            ("claude", vec!["--resume".to_owned(), "abc".to_owned()]),
        ];
        for (tool, expected) in cases {
            assert_eq!(
                interpreter.resume_args_for(&CliToolId::new(tool).unwrap(), "abc"),
                Some(expected),
                "tool: {tool}"
            );
        }
        for tool in ["kimi", "generic", "future-tool"] {
            assert_eq!(
                interpreter.resume_args_for(&CliToolId::new(tool).unwrap(), "abc"),
                None,
                "tool: {tool}"
            );
        }
    }

    #[test]
    fn launch_for_ignores_the_recognition_fallback_and_needs_no_session() {
        let interpreter = CliSessionInterpreter::system();
        let generic_tool = generic_tool();

        assert_eq!(interpreter.launch_for(&generic_tool.id), None);
    }

    #[test]
    fn descriptor_evidence_carries_strong_codex_runtime_and_stays_unknown_for_others() {
        let interpreter = CliSessionInterpreter::system();

        let mut working = session(&["zsh", "codex"]);
        working.title = "qol-tts | Working | fix the queue".to_owned();
        let descriptor = interpreter.describe(&working);
        assert_eq!(descriptor.evidence.runtime, CliRuntimeState::Working);

        let mut awaiting = session(&["zsh", "codex"]);
        awaiting.title = "qol-tts | Action Required | qol-tts".to_owned();
        assert_eq!(
            interpreter.describe(&awaiting).evidence.runtime,
            CliRuntimeState::Unknown
        );

        let mut activity_and_ready = session(&["zsh", "codex"]);
        activity_and_ready.title =
            "qol-tts | Action Required | Ready | gpt-5.6-luna max".to_owned();
        assert_eq!(
            interpreter.describe(&activity_and_ready).evidence.runtime,
            CliRuntimeState::Ready
        );

        let mut ready = session(&["zsh", "codex"]);
        ready.title = "qol-tts | Ready | qol-tts".to_owned();
        assert_eq!(
            interpreter.describe(&ready).evidence.runtime,
            CliRuntimeState::Ready
        );

        let claude = interpreter.describe(&session(&["zsh", "claude"]));
        assert_eq!(claude.evidence.runtime, CliRuntimeState::Unknown);
        assert_eq!(claude.evidence.activity, CliActivityEvidence::default());
        assert_eq!(claude.evidence, CliSessionEvidence::default());
    }

    #[test]
    fn metadata_attachment_never_proves_a_live_viewport() {
        let interpreter = CliSessionInterpreter::system();

        let mut codex = session(&["zsh", "codex"]);
        codex.title = "qol-tts | Working | fix the queue".to_owned();
        let descriptor = interpreter.describe(&codex);
        assert_eq!(descriptor.evidence.runtime, CliRuntimeState::Working);
        assert_eq!(
            descriptor.evidence,
            CliSessionEvidence {
                runtime: CliRuntimeState::Working,
                activity: descriptor.evidence.activity,
            }
        );
        assert_eq!(descriptor.external_id, None);

        let mut claude = session(&["zsh", "claude"]);
        claude.title = "Claude".to_owned();
        assert_eq!(
            interpreter.describe(&claude).evidence.runtime,
            CliRuntimeState::Unknown
        );
    }

    #[test]
    fn classify_screen_routes_by_tool_and_generic_prompt_keeps_viewport_unknown() {
        let interpreter = CliSessionInterpreter::system();

        let codex = session(&["zsh", "codex"]);
        assert_eq!(
            interpreter.classify_screen(&codex, "\u{2728} Working \u{2026} (2s)\nesc to interrupt"),
            CliScreenEvidence {
                viewport: CliViewportState::Live,
                runtime: CliRuntimeState::Working,
            }
        );
        assert_eq!(
            interpreter.classify_screen(&codex, "OpenAI Codex (v0.40)\nTip: Try the Codex App"),
            CliScreenEvidence {
                viewport: CliViewportState::Historical,
                runtime: CliRuntimeState::Unknown,
            }
        );

        let mut shell = session(&["bash"]);
        shell.at_prompt = true;
        assert_eq!(
            interpreter.classify_screen(&shell, "kmrh47@host:~$"),
            CliScreenEvidence {
                viewport: CliViewportState::Unknown,
                runtime: CliRuntimeState::Ready,
            }
        );
        let mut busy_shell = session(&["bash"]);
        busy_shell.at_prompt = false;
        assert_eq!(
            interpreter.classify_screen(&busy_shell, "building..."),
            CliScreenEvidence::default()
        );
    }

    #[test]
    fn generic_descriptor_never_invents_strong_state() {
        let interpreter = CliSessionInterpreter::system();
        let mut shell = session(&["bash"]);
        shell.at_prompt = true;
        shell.title = "Working".to_owned();

        assert_eq!(
            interpreter.describe(&shell).evidence,
            CliSessionEvidence::default()
        );
    }

    fn strategy(
        id: &str,
        label: &str,
        process: &'static str,
        priority: i32,
    ) -> Arc<dyn CliSessionStrategy> {
        Arc::new(NamedStrategy {
            tool: CliTool::new(
                CliToolId::new(id).unwrap(),
                label,
                CliToolColor::new(0x80, 0x80, 0x80),
            ),
            process,
            priority,
        })
    }

    fn session(processes: &[&str]) -> SessionFacts {
        SessionFacts {
            id: SessionId::new(BackendId::new("test").unwrap(), "1").unwrap(),
            root_pid: 1,
            cwd: "/work/project".to_owned(),
            title: "Terminal".to_owned(),
            at_prompt: false,
            reported_cmd: None,
            foreground_basenames: processes
                .iter()
                .map(|process| (*process).to_owned())
                .collect(),
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: None,
        }
    }
}
