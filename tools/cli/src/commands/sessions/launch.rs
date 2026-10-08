use std::collections::BTreeMap;

use anyhow::Result;

use super::agent_policy::{AgentDispatch, DispatchPolicy};
use super::launch_flags::takes_effort;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LaunchKind {
    Fork,
    Spawn,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct LaunchDefaults {
    pub(super) aliases: BTreeMap<String, String>,
    pub(super) fork_model: Option<String>,
    pub(super) fork_effort: Option<String>,
    pub(super) fork_surface: Option<String>,
    pub(super) spawn_effort: Option<String>,
}

impl LaunchDefaults {
    fn model(&self, kind: LaunchKind) -> Option<&str> {
        match kind {
            LaunchKind::Fork => self.fork_model.as_deref(),
            LaunchKind::Spawn => None,
        }
    }

    fn effort(&self, kind: LaunchKind) -> Option<&str> {
        match kind {
            LaunchKind::Fork => self.fork_effort.as_deref(),
            LaunchKind::Spawn => self.spawn_effort.as_deref(),
        }
    }

    fn surface(&self, kind: LaunchKind) -> Option<&str> {
        match kind {
            LaunchKind::Fork => self.fork_surface.as_deref(),
            LaunchKind::Spawn => None,
        }
    }

    fn alias(&self, value: Option<String>) -> Option<String> {
        value.map(|value| self.aliases.get(&value).cloned().unwrap_or(value))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct LaunchChoice {
    pub(super) tool: Option<String>,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) surface: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Launch {
    pub(super) tool: String,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) surface: Option<String>,
}

pub(super) fn resolve(
    kind: LaunchKind,
    explicit: LaunchChoice,
    defaults: &LaunchDefaults,
    dispatch: &AgentDispatch,
) -> Result<Launch> {
    let tool = defaults.alias(explicit.tool);
    let chosen = defaults.alias(explicit.model);
    let model = match tool.as_deref() {
        _ if chosen.is_some() || dispatch.is_constrained() => chosen,
        Some(tool) => model_for_tool(tool, defaults.model(kind), &dispatch.policy),
        None => defaults.model(kind).map(str::to_owned),
    };
    let tool = match tool {
        Some(tool) => tool,
        None => dispatch.resolve_launch_tool(None, model.as_deref())?,
    };
    let effort = defaults.alias(explicit.effort).or_else(|| {
        defaults
            .effort(kind)
            .filter(|_| takes_effort(&tool))
            .map(str::to_owned)
    });
    let surface = defaults
        .alias(explicit.surface)
        .or_else(|| defaults.surface(kind).map(str::to_owned));
    Ok(Launch {
        tool,
        model,
        effort,
        surface,
    })
}

pub(super) fn inherit_model(
    explicit: &mut LaunchChoice,
    inherited: Option<String>,
    defaults: &LaunchDefaults,
    dispatch: &AgentDispatch,
) {
    if explicit.model.is_some() || dispatch.is_constrained() {
        return;
    }
    let Some(model) = inherited else {
        return;
    };
    let tool = match defaults.alias(explicit.tool.clone()) {
        Some(tool) => tool,
        None => match dispatch.resolve_launch_tool(None, Some(&model)) {
            Ok(tool) => tool,
            Err(_) => return,
        },
    };
    if dispatch.admit_launch(&tool, Some(&model)).is_ok() {
        explicit.model = Some(model);
    }
}

fn model_for_tool(tool: &str, default: Option<&str>, policy: &DispatchPolicy) -> Option<String> {
    let candidates = [default, policy.default_model.as_deref()];
    let Some(declared) = policy.tool_models.get(tool) else {
        return candidates.into_iter().flatten().next().map(str::to_owned);
    };
    candidates
        .into_iter()
        .flatten()
        .find(|model| declared.iter().any(|entry| entry == model))
        .or(declared.first().map(String::as_str))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::sessions::agent_policy::AssignmentRequest;

    fn dispatch() -> AgentDispatch {
        AgentDispatch::new(
            DispatchPolicy {
                default_model: Some("flash".to_owned()),
                allowed_models: vec![
                    "opus".to_owned(),
                    "sonnet".to_owned(),
                    "glm".to_owned(),
                    "flash".to_owned(),
                ],
                tool_models: BTreeMap::from([
                    (
                        "claude".to_owned(),
                        vec!["opus".to_owned(), "sonnet".to_owned()],
                    ),
                    ("pi".to_owned(), vec!["glm".to_owned(), "flash".to_owned()]),
                    ("codex".to_owned(), vec!["astra".to_owned()]),
                ]),
                ..DispatchPolicy::default()
            },
            AssignmentRequest::default(),
        )
    }

    fn defaults() -> LaunchDefaults {
        LaunchDefaults {
            aliases: BTreeMap::from([
                ("cc".to_owned(), "claude".to_owned()),
                ("win".to_owned(), "os-window".to_owned()),
            ]),
            fork_model: Some("opus".to_owned()),
            fork_effort: Some("high".to_owned()),
            fork_surface: Some("tab".to_owned()),
            spawn_effort: Some("medium".to_owned()),
        }
    }

    fn ask(
        tool: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
        surface: Option<&str>,
    ) -> LaunchChoice {
        LaunchChoice {
            tool: tool.map(str::to_owned),
            model: model.map(str::to_owned),
            effort: effort.map(str::to_owned),
            surface: surface.map(str::to_owned),
        }
    }

    fn launch(
        tool: &str,
        model: Option<&str>,
        effort: Option<&str>,
        surface: Option<&str>,
    ) -> Launch {
        Launch {
            tool: tool.to_owned(),
            model: model.map(str::to_owned),
            effort: effort.map(str::to_owned),
            surface: surface.map(str::to_owned),
        }
    }

    #[test]
    fn named_values_fill_their_slot_through_aliases_and_defaults_fill_the_rest() {
        let cases = [
            (
                LaunchKind::Fork,
                ask(None, None, None, None),
                launch("claude", Some("opus"), Some("high"), Some("tab")),
            ),
            (
                LaunchKind::Fork,
                ask(None, None, None, Some("win")),
                launch("claude", Some("opus"), Some("high"), Some("os-window")),
            ),
            (
                LaunchKind::Fork,
                ask(None, Some("sonnet"), Some("low"), None),
                launch("claude", Some("sonnet"), Some("low"), Some("tab")),
            ),
            (
                LaunchKind::Fork,
                ask(Some("pi"), None, None, None),
                launch("pi", Some("flash"), Some("high"), Some("tab")),
            ),
            (
                LaunchKind::Fork,
                ask(Some("codex"), None, None, None),
                launch("codex", Some("astra"), None, Some("tab")),
            ),
            (
                LaunchKind::Spawn,
                ask(None, None, None, None),
                launch("pi", None, Some("medium"), None),
            ),
            (
                LaunchKind::Spawn,
                ask(Some("cc"), None, None, None),
                launch("claude", Some("opus"), Some("medium"), None),
            ),
            (
                LaunchKind::Spawn,
                ask(None, Some("sonnet"), None, Some("tab")),
                launch("claude", Some("sonnet"), Some("medium"), Some("tab")),
            ),
        ];
        for (kind, choice, expected) in cases {
            let resolved = resolve(kind, choice.clone(), &defaults(), &dispatch()).unwrap();
            assert_eq!(resolved, expected, "{kind:?} {choice:?}");
        }
    }

    #[test]
    fn a_fork_inherits_the_parent_model_only_when_nothing_names_one_and_the_pair_is_admitted() {
        let cases = [
            (ask(None, None, None, None), Some("sonnet"), Some("sonnet")),
            (ask(None, None, None, None), Some("glm"), Some("glm")),
            (
                ask(None, Some("opus"), None, None),
                Some("sonnet"),
                Some("opus"),
            ),
            (ask(None, None, None, None), Some("haiku"), None),
            (ask(Some("pi"), None, None, None), Some("sonnet"), None),
            (
                ask(Some("cc"), None, None, None),
                Some("sonnet"),
                Some("sonnet"),
            ),
            (ask(None, None, None, None), None, None),
        ];
        for (choice, inherited, expected) in cases {
            let mut inheriting = choice.clone();
            inherit_model(
                &mut inheriting,
                inherited.map(str::to_owned),
                &defaults(),
                &dispatch(),
            );
            let expected = expected.or(choice.model.as_deref());
            assert_eq!(
                inheriting.model.as_deref(),
                expected,
                "{choice:?} {inherited:?}"
            );
        }
    }

    #[test]
    fn a_constrained_fork_never_inherits_the_parent_model() {
        let constrained = AgentDispatch::new(
            dispatch().policy,
            AssignmentRequest {
                agent_profile: Some("planner".to_owned()),
                ..AssignmentRequest::default()
            },
        );
        let mut choice = ask(None, None, None, None);
        inherit_model(
            &mut choice,
            Some("sonnet".to_owned()),
            &defaults(),
            &constrained,
        );
        assert_eq!(choice.model, None);
    }
}
