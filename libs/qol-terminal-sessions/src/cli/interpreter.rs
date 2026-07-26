use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::SessionFacts;

use super::builtins::GenericStrategy;
use super::{
    CliSessionChangeHandler, CliSessionDescriptor, CliSessionStrategy, CliSessionSubscription,
    CliSessionSubscriptionError, CliToolId,
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
        let descriptor = strategy.describe(session);
        qol_runtime::probe!(
            "CLI_SESSION_INTERPRETATION",
            "event=described tool={} terminal_backend={}",
            descriptor.tool.id,
            session.id.backend()
        );
        descriptor
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
        generic_tool, CliSessionDescriptor, CliSessionInterpreter, CliSessionStrategy, CliTool,
        CliToolColor, CliToolId,
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
        }
    }
}
