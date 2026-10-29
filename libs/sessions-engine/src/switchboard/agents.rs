use std::path::Path;

use serde_json::{json, Map, Value};

use super::hooks::{FORK_CONTEXT, POST_AGENT, PRE_AGENT, WORKTREE_CREATE};
use super::resume::FORK_AGENT;
use crate::agent_policy::{enforce_allowed_model, enforce_tool_model, DispatchPolicy};
use crate::launch::LaunchDefaults;

pub(super) const AGENT: &str = "switchboard";
const CLAUDE: &str = "claude";
const SESSIONS_TOOLS: &str = "mcp__sessions__*";
const AGENT_TOOL_FOR_FORKS: &str = "Agent(project-fork)";
const PROMPT_CACHE_TTL: &str = "1h";
pub(super) const CONTEXT_SLOTS: usize = super::context::PART_SLOTS;

pub(super) const FORK_PROMPT: &str = "You are a fork of the switchboard: you own one piece of work end to end, in the project folder named on the first line of your task. The project's instructions, memory index and delivery rules are added when you start; follow them as you would in your own session. Each Bash call starts in the project folder, so once you make a worktree, prefix commands with `cd <worktree> &&`. Read before editing, verify before claiming, and report failures faithfully. Messages the switchboard relays are the user's own words; the user approves risky steps in your own view. Your final message is your report to the switchboard: one or two lines with the pull request number and its state, or the blocker. Never use session_send. A question only the user can decide ends your turn, stating the question and your recommended answer. Never use `qol-cli-sessions park`. To watch a pull request, run its watcher command without the park prefix through Bash with run_in_background true, then end your turn saying what you wait for; Claude Code wakes you with the exit code and output.";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct ForkAgent {
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
}

impl ForkAgent {
    pub(super) fn resolve(defaults: &LaunchDefaults, policy: &DispatchPolicy) -> Self {
        let model = defaults.alias(defaults.fork_model.clone()).filter(|model| {
            enforce_allowed_model(model, &policy.allowed_models).is_ok()
                && enforce_tool_model(CLAUDE, model, &policy.tool_models).is_ok()
        });
        let effort = defaults.alias(defaults.fork_effort.clone());
        Self { model, effort }
    }

    pub(super) fn system() -> Self {
        let defaults = crate::spawn::config_launch_defaults();
        let policy = crate::spawn::config_dispatch_policy();
        match (defaults, policy) {
            (Ok(defaults), Ok(policy)) => Self::resolve(&defaults, &policy),
            (defaults, policy) => {
                for error in [defaults.err(), policy.err()].into_iter().flatten() {
                    crate::cli::warn(format!("forks use the default model: {error:#}"));
                }
                Self::default()
            }
        }
    }
}

pub(super) fn admit_model(model: &str) -> anyhow::Result<()> {
    let defaults = crate::spawn::config_launch_defaults()?;
    let policy = crate::spawn::config_dispatch_policy()?;
    let model = defaults.alias(Some(model.to_owned())).unwrap_or_default();
    enforce_allowed_model(&model, &policy.allowed_models)?;
    enforce_tool_model(CLAUDE, &model, &policy.tool_models)
}

pub(super) fn agents_json(fork: &ForkAgent) -> String {
    let mut project_fork = Map::new();
    project_fork.insert(
        "description".to_owned(),
        json!("One piece of work in one project, owned end to end by a fork of the switchboard. Start it with a prompt whose first line is `Project: <absolute project folder>`."),
    );
    project_fork.insert("prompt".to_owned(), json!(FORK_PROMPT));
    if let Some(model) = &fork.model {
        project_fork.insert("model".to_owned(), json!(model));
    }
    if let Some(effort) = &fork.effort {
        project_fork.insert("effort".to_owned(), json!(effort));
    }
    project_fork.insert("isolation".to_owned(), json!("worktree"));
    project_fork.insert("background".to_owned(), json!(true));
    json!({
        AGENT: {
            "description": "The user's one control chat: it starts forks and talks to terminal sessions.",
            "prompt": "",
            "tools": [AGENT_TOOL_FOR_FORKS, "SendMessage", "TaskStop", "ToolSearch", SESSIONS_TOOLS],
        },
        FORK_AGENT: Value::Object(project_fork),
    })
    .to_string()
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn hook_command(program: &Path, hook: &str) -> String {
    format!("{} sessions switchboard hook {hook}", shell_quote(program))
}

fn entry(matcher: Option<&str>, command: String) -> Value {
    let mut entry = json!({"hooks": [{"type": "command", "command": command}]});
    if let Some(matcher) = matcher {
        entry["matcher"] = json!(matcher);
    }
    entry
}

pub(super) fn settings_json(program: &Path) -> String {
    let command = |hook: &str| hook_command(program, hook);
    let context: Vec<Value> = (0..CONTEXT_SLOTS)
        .map(|part| entry(Some(FORK_AGENT), command(&format!("{FORK_CONTEXT} {part}"))))
        .collect();
    json!({
        "subagentPromptCacheTtl": PROMPT_CACHE_TTL,
        "hooks": {
            "PreToolUse": [entry(Some("Agent"), command(PRE_AGENT))],
            "PostToolUse": [entry(Some("Agent"), command(POST_AGENT))],
            "PostToolUseFailure": [entry(Some("Agent"), command(POST_AGENT))],
            "WorktreeCreate": [entry(None, command(WORKTREE_CREATE))],
            "SubagentStart": context,
        },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_policy::DispatchPolicy;
    use std::collections::BTreeMap;

    fn defaults(model: Option<&str>, effort: Option<&str>) -> LaunchDefaults {
        LaunchDefaults {
            aliases: BTreeMap::from([("fast".to_owned(), "claude-opus-5-5".to_owned())]),
            fork_model: model.map(str::to_owned),
            fork_effort: effort.map(str::to_owned),
            ..LaunchDefaults::default()
        }
    }

    fn policy(tool_models: &[(&str, &[&str])], allowed: &[&str]) -> DispatchPolicy {
        DispatchPolicy {
            allowed_models: allowed.iter().map(|m| (*m).to_owned()).collect(),
            tool_models: tool_models
                .iter()
                .map(|(tool, models)| {
                    (
                        (*tool).to_owned(),
                        models.iter().map(|m| (*m).to_owned()).collect(),
                    )
                })
                .collect(),
            ..DispatchPolicy::default()
        }
    }

    #[test]
    fn the_fork_model_goes_through_aliases_and_both_admission_lists() {
        let open = policy(&[], &[]);
        let fork = ForkAgent::resolve(&defaults(Some("fast"), Some("high")), &open);
        assert_eq!(fork.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(fork.effort.as_deref(), Some("high"));
        let declared = policy(&[("claude", &["claude-opus-*"])], &["claude-*"]);
        assert!(ForkAgent::resolve(&defaults(Some("fast"), None), &declared)
            .model
            .is_some());
        let other_tool = policy(&[("pi", &["claude-opus-*"])], &[]);
        assert_eq!(
            ForkAgent::resolve(&defaults(Some("fast"), None), &other_tool).model,
            None
        );
        let wrong_model = policy(&[("claude", &["claude-haiku-*"])], &[]);
        assert_eq!(
            ForkAgent::resolve(&defaults(Some("fast"), None), &wrong_model).model,
            None
        );
        let not_allowed = policy(&[], &["claude-sonnet-*"]);
        assert_eq!(
            ForkAgent::resolve(&defaults(Some("fast"), None), &not_allowed).model,
            None
        );
        assert_eq!(
            ForkAgent::resolve(&defaults(None, None), &open),
            ForkAgent::default()
        );
    }

    #[test]
    fn the_agents_json_defines_the_lead_and_the_worktree_pinned_background_fork() {
        let fork = ForkAgent {
            model: Some("claude-opus-5-5".to_owned()),
            effort: Some("high".to_owned()),
        };
        let agents: Value = serde_json::from_str(&agents_json(&fork)).unwrap();
        let lead = &agents["switchboard"];
        assert_eq!(lead["prompt"], "");
        assert!(lead.get("model").is_none());
        assert_eq!(
            lead["tools"],
            json!([
                "Agent(project-fork)",
                "SendMessage",
                "TaskStop",
                "ToolSearch",
                "mcp__sessions__*"
            ])
        );
        let worker = &agents["project-fork"];
        assert_eq!(worker["prompt"], FORK_PROMPT);
        assert_eq!(worker["model"], "claude-opus-5-5");
        assert_eq!(worker["effort"], "high");
        assert_eq!(worker["isolation"], "worktree");
        assert_eq!(worker["background"], true);
        assert!(agents.get("fork").is_none());
        let bare: Value = serde_json::from_str(&agents_json(&ForkAgent::default())).unwrap();
        assert!(bare["project-fork"].get("model").is_none());
        assert!(bare["project-fork"].get("effort").is_none());
    }

    #[test]
    fn the_fork_prompt_carries_the_rules_the_subagent_prompt_replaces() {
        for needle in [
            "cd <worktree> &&",
            "Never use session_send",
            "Never use `qol-cli-sessions park`",
            "run_in_background true",
            "recommended answer",
            "report failures faithfully",
        ] {
            assert!(FORK_PROMPT.contains(needle), "{needle}");
        }
    }

    #[test]
    fn the_settings_carry_the_cache_ttl_and_every_hook() {
        let program = Path::new("/plugins/it's/qol-cli-sessions");
        let settings: Value = serde_json::from_str(&settings_json(program)).unwrap();
        assert_eq!(settings["subagentPromptCacheTtl"], "1h");
        let hooks = &settings["hooks"];
        let command_of = |entry: &Value| entry["hooks"][0]["command"].as_str().unwrap().to_owned();
        let prefix = "'/plugins/it'\\''s/qol-cli-sessions' sessions switchboard hook";
        for (event, hook) in [
            ("PreToolUse", "pre-agent"),
            ("PostToolUse", "post-agent"),
            ("PostToolUseFailure", "post-agent"),
        ] {
            assert_eq!(hooks[event].as_array().unwrap().len(), 1, "{event}");
            assert_eq!(hooks[event][0]["matcher"], "Agent", "{event}");
            assert_eq!(command_of(&hooks[event][0]), format!("{prefix} {hook}"));
        }
        assert_eq!(hooks["WorktreeCreate"][0].get("matcher"), None);
        assert_eq!(
            command_of(&hooks["WorktreeCreate"][0]),
            format!("{prefix} worktree-create")
        );
        let context = hooks["SubagentStart"].as_array().unwrap();
        assert_eq!(context.len(), 4);
        for (part, entry) in context.iter().enumerate() {
            assert_eq!(entry["matcher"], "project-fork");
            assert_eq!(command_of(entry), format!("{prefix} fork-context {part}"));
        }
        assert!(settings.get("alwaysThinkingEnabled").is_none());
    }
}
