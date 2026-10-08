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

fn model_for_tool(tool: &str, default: Option<&str>, policy: &DispatchPolicy) -> Option<String> {
    let default = default.or(policy.default_model.as_deref());
    let Some(declared) = policy.tool_models.get(tool) else {
        return default.map(str::to_owned);
    };
    default
        .filter(|model| declared.iter().any(|entry| entry == model))
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
                allowed_models: vec!["opus".to_owned(), "sonnet".to_owned(), "flash".to_owned()],
                tool_models: BTreeMap::from([
                    (
                        "claude".to_owned(),
                        vec!["opus".to_owned(), "sonnet".to_owned()],
                    ),
                    ("pi".to_owned(), vec!["flash".to_owned()]),
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
}
