use std::collections::BTreeMap;

use anyhow::{bail, Result};

use super::agent_policy::{AgentDispatch, DispatchPolicy};
use super::launch_flags::{takes_effort, EFFORT_LEVELS};
use super::spawn::{SURFACE_OS_WINDOW, SURFACE_TAB};

pub(super) fn is_pick(argument: &str) -> bool {
    argument.len() > 1
        && argument.starts_with('-')
        && !argument.starts_with("--")
        && argument != "-h"
}

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
    picks: &[String],
    defaults: &LaunchDefaults,
    dispatch: &AgentDispatch,
) -> Result<Launch> {
    let picked = classify(picks, &defaults.aliases, &dispatch.policy.tool_models)?;
    let tool = explicit.tool.or(picked.tool);
    let chosen = explicit.model.or(picked.model);
    let model = match tool.as_deref() {
        _ if chosen.is_some() || dispatch.is_constrained() => chosen,
        Some(tool) => model_for_tool(tool, defaults.model(kind), &dispatch.policy),
        None => defaults.model(kind).map(str::to_owned),
    };
    let tool = match tool {
        Some(tool) => tool,
        None => dispatch.resolve_launch_tool(None, model.as_deref())?,
    };
    let effort = explicit.effort.or(picked.effort).or_else(|| {
        defaults
            .effort(kind)
            .filter(|_| takes_effort(&tool))
            .map(str::to_owned)
    });
    let surface = explicit
        .surface
        .or(picked.surface)
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

fn classify(
    picks: &[String],
    aliases: &BTreeMap<String, String>,
    tool_models: &BTreeMap<String, Vec<String>>,
) -> Result<LaunchChoice> {
    let mut choice = LaunchChoice::default();
    for raw in picks {
        let token = raw.trim_start_matches('-');
        let value = aliases.get(token).map(String::as_str).unwrap_or(token);
        let (slot, name) = if value == SURFACE_TAB || value == SURFACE_OS_WINDOW {
            (&mut choice.surface, "surface")
        } else if EFFORT_LEVELS.contains(&value) {
            (&mut choice.effort, "effort")
        } else if tool_models.contains_key(value) {
            (&mut choice.tool, "harness")
        } else if tool_models.values().flatten().any(|model| model == value) {
            (&mut choice.model, "model")
        } else {
            bail!(
                "unknown pick `{raw}`; a pick is a surface ({SURFACE_TAB}, {SURFACE_OS_WINDOW}), an effort ({}), a harness or model from tool_models, or an alias from [aliases] in sessions.toml{}",
                EFFORT_LEVELS.join(", "),
                alias_catalog(aliases)
            );
        };
        if let Some(previous) = slot {
            bail!("`{raw}` picks a second {name}; `{previous}` is already picked");
        }
        *slot = Some(value.to_owned());
    }
    Ok(choice)
}

fn alias_catalog(aliases: &BTreeMap<String, String>) -> String {
    if aliases.is_empty() {
        return String::new();
    }
    let entries = aliases
        .iter()
        .map(|(alias, value)| format!("{alias}={value}"))
        .collect::<Vec<_>>();
    format!(" ({})", entries.join(", "))
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

    fn choice(
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

    fn picks(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|token| (*token).to_owned()).collect()
    }

    #[test]
    fn picks_fill_only_the_slots_they_name_and_defaults_fill_the_rest() {
        let cases: &[(LaunchKind, &[&str], Launch)] = &[
            (
                LaunchKind::Fork,
                &[],
                choice("claude", Some("opus"), Some("high"), Some("tab")),
            ),
            (
                LaunchKind::Fork,
                &["-win"],
                choice("claude", Some("opus"), Some("high"), Some("os-window")),
            ),
            (
                LaunchKind::Fork,
                &["-low", "-sonnet"],
                choice("claude", Some("sonnet"), Some("low"), Some("tab")),
            ),
            (
                LaunchKind::Fork,
                &["-pi"],
                choice("pi", Some("flash"), Some("high"), Some("tab")),
            ),
            (
                LaunchKind::Fork,
                &["-pi", "-max"],
                choice("pi", Some("flash"), Some("max"), Some("tab")),
            ),
            (
                LaunchKind::Spawn,
                &[],
                choice("pi", None, Some("medium"), None),
            ),
            (
                LaunchKind::Spawn,
                &["-cc"],
                choice("claude", Some("opus"), Some("medium"), None),
            ),
            (
                LaunchKind::Spawn,
                &["sonnet", "tab"],
                choice("claude", Some("sonnet"), Some("medium"), Some("tab")),
            ),
        ];
        for (kind, tokens, expected) in cases {
            let resolved = resolve(
                *kind,
                LaunchChoice::default(),
                &picks(tokens),
                &defaults(),
                &dispatch(),
            )
            .unwrap();
            assert_eq!(&resolved, expected, "{kind:?} {tokens:?}");
        }
    }

    #[test]
    fn an_explicit_value_beats_a_pick_and_a_default() {
        let explicit = LaunchChoice {
            effort: Some("xhigh".to_owned()),
            ..LaunchChoice::default()
        };
        let resolved = resolve(
            LaunchKind::Fork,
            explicit,
            &picks(&["-low"]),
            &defaults(),
            &dispatch(),
        )
        .unwrap();
        assert_eq!(resolved.effort.as_deref(), Some("xhigh"));
    }

    #[test]
    fn an_effort_default_never_reaches_a_harness_without_an_effort_flag() {
        let mut dispatch_policy = dispatch().policy.clone();
        dispatch_policy
            .tool_models
            .insert("codex".to_owned(), vec!["astra".to_owned()]);
        let dispatch = AgentDispatch::new(dispatch_policy, AssignmentRequest::default());
        let resolved = resolve(
            LaunchKind::Fork,
            LaunchChoice::default(),
            &picks(&["-codex"]),
            &defaults(),
            &dispatch,
        )
        .unwrap();
        assert_eq!(resolved, choice("codex", Some("astra"), None, Some("tab")));
    }

    #[test]
    fn unknown_and_doubled_picks_are_refused_by_name() {
        let cases: &[(&[&str], &str)] = &[
            (&["-nope"], "unknown pick `-nope`"),
            (&["-cc", "-pi"], "picks a second harness"),
            (&["-low", "-high"], "picks a second effort"),
            (&["-win", "-tab"], "picks a second surface"),
            (&["-opus", "-sonnet"], "picks a second model"),
        ];
        for (tokens, expected) in cases {
            let error = resolve(
                LaunchKind::Fork,
                LaunchChoice::default(),
                &picks(tokens),
                &defaults(),
                &dispatch(),
            )
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{tokens:?}: {error}");
        }
    }
}
