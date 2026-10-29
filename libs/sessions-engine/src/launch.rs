use std::collections::BTreeMap;

use anyhow::Result;

use super::agent_policy::{model_declared, AgentDispatch, DispatchPolicy};
use super::launch_flags::{takes_effort, takes_remote_control};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LaunchKind {
    Fork,
    Spawn,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HarnessDefaults {
    pub(super) tool: Option<String>,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct LaunchDefaults {
    pub(super) aliases: BTreeMap<String, String>,
    pub(super) spawn_defaults: BTreeMap<String, HarnessDefaults>,
    pub(super) fork_model: Option<String>,
    pub(super) fork_effort: Option<String>,
    pub(super) fork_surface: Option<String>,
    pub(super) spawn_effort: Option<String>,
    pub(super) fork_remote_control: bool,
    pub(super) spawn_remote_control: bool,
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

    fn remote_control(&self, kind: LaunchKind) -> bool {
        match kind {
            LaunchKind::Fork => self.fork_remote_control,
            LaunchKind::Spawn => self.spawn_remote_control,
        }
    }

    pub(super) fn alias(&self, value: Option<String>) -> Option<String> {
        value.map(|value| self.aliases.get(&value).cloned().unwrap_or(value))
    }

    fn harness(
        &self,
        kind: LaunchKind,
        caller_tool: Option<&str>,
        dispatch: &AgentDispatch,
    ) -> Option<(String, &HarnessDefaults)> {
        if kind != LaunchKind::Spawn || dispatch.is_constrained() {
            return None;
        }
        let caller_tool = caller_tool?;
        let entry = self.spawn_defaults.get(caller_tool)?;
        let tool = self
            .alias(entry.tool.clone())
            .unwrap_or_else(|| caller_tool.to_owned());
        Some((tool, entry))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct LaunchChoice {
    pub(super) tool: Option<String>,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) surface: Option<String>,
    pub(super) remote_control: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Launch {
    pub(super) tool: String,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) surface: Option<String>,
    pub(super) remote_control: bool,
}

pub(super) fn resolve(
    kind: LaunchKind,
    explicit: LaunchChoice,
    defaults: &LaunchDefaults,
    dispatch: &AgentDispatch,
    caller_tool: Option<&str>,
) -> Result<Launch> {
    let harness = defaults.harness(kind, caller_tool, dispatch);
    let chosen = defaults.alias(explicit.model);
    let tool = defaults.alias(explicit.tool).or_else(|| {
        harness
            .as_ref()
            .filter(|_| chosen.is_none())
            .map(|(tool, _)| tool.clone())
    });
    let entry_for = |tool: &str| {
        harness
            .as_ref()
            .filter(|(harness_tool, _)| harness_tool == tool)
            .map(|(_, entry)| *entry)
    };
    let model = match tool.as_deref() {
        _ if chosen.is_some() || dispatch.is_constrained() => chosen,
        Some(tool) => entry_for(tool)
            .and_then(|entry| defaults.alias(entry.model.clone()))
            .or_else(|| model_for_tool(tool, defaults.model(kind), &dispatch.policy)),
        None => defaults.model(kind).map(str::to_owned),
    };
    let tool = match tool {
        Some(tool) => tool,
        None => dispatch.resolve_launch_tool(None, model.as_deref())?,
    };
    let effort = defaults.alias(explicit.effort).or_else(|| {
        entry_for(&tool)
            .and_then(|entry| defaults.alias(entry.effort.clone()))
            .or_else(|| defaults.effort(kind).map(str::to_owned))
            .filter(|_| takes_effort(&tool))
    });
    let surface = defaults
        .alias(explicit.surface)
        .or_else(|| defaults.surface(kind).map(str::to_owned));
    let remote_control = explicit
        .remote_control
        .unwrap_or_else(|| defaults.remote_control(kind) && takes_remote_control(&tool));
    Ok(Launch {
        tool,
        model,
        effort,
        surface,
        remote_control,
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
        .find(|model| model_declared(declared, model))
        .or(declared
            .iter()
            .map(String::as_str)
            .find(|entry| !entry.ends_with('*')))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_policy::AssignmentRequest;

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
            spawn_defaults: BTreeMap::new(),
            fork_remote_control: false,
            spawn_remote_control: false,
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
            remote_control: None,
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
            remote_control: false,
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
            let resolved = resolve(kind, choice.clone(), &defaults(), &dispatch(), None).unwrap();
            assert_eq!(resolved, expected, "{kind:?} {choice:?}");
        }
    }

    #[test]
    fn a_spawn_takes_the_calling_harness_defaults_for_every_slot_it_leaves_out() {
        let configured = LaunchDefaults {
            spawn_defaults: BTreeMap::from([
                (
                    "claude".to_owned(),
                    HarnessDefaults {
                        tool: None,
                        model: Some("opus".to_owned()),
                        effort: Some("high".to_owned()),
                    },
                ),
                (
                    "pi".to_owned(),
                    HarnessDefaults {
                        tool: Some("cc".to_owned()),
                        model: None,
                        effort: Some("low".to_owned()),
                    },
                ),
            ]),
            ..defaults()
        };
        let cases = [
            (
                LaunchKind::Spawn,
                Some("claude"),
                ask(None, None, None, None),
                launch("claude", Some("opus"), Some("high"), None),
            ),
            (
                LaunchKind::Spawn,
                Some("claude"),
                ask(None, Some("sonnet"), None, None),
                launch("claude", Some("sonnet"), Some("high"), None),
            ),
            (
                LaunchKind::Spawn,
                Some("claude"),
                ask(None, None, Some("low"), None),
                launch("claude", Some("opus"), Some("low"), None),
            ),
            (
                LaunchKind::Spawn,
                Some("claude"),
                ask(Some("pi"), None, None, None),
                launch("pi", Some("flash"), Some("medium"), None),
            ),
            (
                LaunchKind::Spawn,
                Some("claude"),
                ask(None, Some("glm"), None, None),
                launch("pi", Some("glm"), Some("medium"), None),
            ),
            (
                LaunchKind::Spawn,
                Some("pi"),
                ask(None, None, None, None),
                launch("claude", Some("opus"), Some("low"), None),
            ),
            (
                LaunchKind::Spawn,
                Some("codex"),
                ask(None, None, None, None),
                launch("pi", None, Some("medium"), None),
            ),
            (
                LaunchKind::Spawn,
                None,
                ask(None, None, None, None),
                launch("pi", None, Some("medium"), None),
            ),
            (
                LaunchKind::Fork,
                Some("pi"),
                ask(None, None, None, None),
                launch("claude", Some("opus"), Some("high"), Some("tab")),
            ),
        ];
        for (kind, caller, choice, expected) in cases {
            let resolved = resolve(kind, choice.clone(), &configured, &dispatch(), caller).unwrap();
            assert_eq!(resolved, expected, "{kind:?} {caller:?} {choice:?}");
        }
    }

    #[test]
    fn remote_control_follows_its_launch_kind_default_unless_named_and_skips_tools_without_it() {
        let configured = LaunchDefaults {
            fork_remote_control: true,
            ..defaults()
        };
        let cases = [
            (LaunchKind::Fork, ask(None, None, None, None), None, true),
            (
                LaunchKind::Fork,
                ask(None, None, None, None),
                Some(false),
                false,
            ),
            (
                LaunchKind::Spawn,
                ask(Some("cc"), None, None, None),
                None,
                false,
            ),
            (
                LaunchKind::Spawn,
                ask(Some("cc"), None, None, None),
                Some(true),
                true,
            ),
            (
                LaunchKind::Fork,
                ask(Some("codex"), None, None, None),
                None,
                false,
            ),
            (
                LaunchKind::Fork,
                ask(Some("codex"), None, None, None),
                Some(true),
                true,
            ),
        ];
        for (kind, mut choice, explicit, expected) in cases {
            choice.remote_control = explicit;
            let resolved = resolve(kind, choice.clone(), &configured, &dispatch(), None).unwrap();
            assert_eq!(resolved.remote_control, expected, "{kind:?} {choice:?}");
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
