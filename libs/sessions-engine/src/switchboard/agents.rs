use std::path::Path;

use anyhow::Context;
use serde_json::{json, Map, Value};

use super::hooks::{AGENT_ROWS, FORK_CONTEXT, PRE_AGENT};
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

fn definitions(fork: &ForkAgent) -> Map<String, Value> {
    let mut lead = Map::new();
    lead.insert(
        "description".to_owned(),
        json!("The user's one control chat: it starts forks and talks to terminal sessions."),
    );
    lead.insert("prompt".to_owned(), json!(""));
    lead.insert(
        "tools".to_owned(),
        json!([
            AGENT_TOOL_FOR_FORKS,
            "SendMessage",
            "TaskStop",
            "ToolSearch",
            SESSIONS_TOOLS
        ]),
    );
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
    project_fork.insert("background".to_owned(), json!(true));
    let mut definitions = Map::new();
    definitions.insert(AGENT.to_owned(), Value::Object(lead));
    definitions.insert(FORK_AGENT.to_owned(), Value::Object(project_fork));
    definitions
}

pub(super) fn agents_json(fork: &ForkAgent) -> String {
    Value::Object(definitions(fork)).to_string()
}

pub(super) fn agent_files(fork: &ForkAgent) -> Vec<(String, String)> {
    definitions(fork)
        .into_iter()
        .map(|(name, definition)| {
            let mut contents = format!("---\nname: {}\n", Value::String(name.clone()));
            let mut prompt = "";
            if let Value::Object(fields) = &definition {
                for (key, value) in fields {
                    if key == "prompt" {
                        prompt = value.as_str().unwrap_or_default();
                    } else {
                        contents.push_str(&format!("{key}: {value}\n"));
                    }
                }
            }
            contents.push_str("---\n");
            contents.push_str(prompt);
            (format!("{name}.md"), contents)
        })
        .collect()
}

pub(super) fn write_agent_files(dir: &Path, fork: &ForkAgent) -> anyhow::Result<()> {
    let agents = dir.join(".claude").join("agents");
    std::fs::create_dir_all(&agents)
        .with_context(|| format!("failed to create {}", agents.display()))?;
    for (file, contents) in agent_files(fork) {
        let path = agents.join(file);
        std::fs::write(&path, contents)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn hook_command(program: &Path, hook: &str) -> String {
    format!("{} sessions switchboard hook {hook}", shell_quote(program))
}

fn entry(matcher: &str, command: String) -> Value {
    json!({"matcher": matcher, "hooks": [{"type": "command", "command": command}]})
}

pub(super) fn settings_json(program: &Path) -> String {
    let command = |hook: &str| hook_command(program, hook);
    let context: Vec<Value> = (0..CONTEXT_SLOTS)
        .map(|part| entry(FORK_AGENT, command(&format!("{FORK_CONTEXT} {part}"))))
        .collect();
    json!({
        "subagentPromptCacheTtl": PROMPT_CACHE_TTL,
        "subagentStatusLine": {"type": "command", "command": command(AGENT_ROWS)},
        "hooks": {
            "PreToolUse": [entry("Agent", command(PRE_AGENT))],
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
    fn the_agents_json_defines_the_lead_and_the_unisolated_background_fork() {
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
        assert!(worker.get("isolation").is_none(), "{worker}");
        assert_eq!(worker["background"], true);
        assert!(agents.get("fork").is_none());
        let bare: Value = serde_json::from_str(&agents_json(&ForkAgent::default())).unwrap();
        assert!(bare["project-fork"].get("model").is_none());
        assert!(bare["project-fork"].get("effort").is_none());
    }

    fn tuned_fork() -> ForkAgent {
        ForkAgent {
            model: Some("claude-opus-5-5".to_owned()),
            effort: Some("high".to_owned()),
        }
    }

    #[test]
    fn the_agent_files_render_each_definition_as_json_quoted_frontmatter() {
        let files = agent_files(&tuned_fork());
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["switchboard.md", "project-fork.md"]);
        let lead = &files[0].1;
        assert!(lead.starts_with("---\nname: \"switchboard\"\n"), "{lead}");
        assert!(
            lead.contains("\ndescription: \"The user's one control chat"),
            "{lead}"
        );
        assert!(
            lead.contains("\ntools: [\"Agent(project-fork)\",\"SendMessage\",\"TaskStop\",\"ToolSearch\",\"mcp__sessions__*\"]\n"),
            "{lead}"
        );
        assert!(!lead.contains("prompt:"), "{lead}");
        assert!(lead.ends_with("\n---\n"), "{lead}");
        let worker = &files[1].1;
        assert!(
            worker.starts_with("---\nname: \"project-fork\"\n"),
            "{worker}"
        );
        for line in [
            "\nmodel: \"claude-opus-5-5\"\n",
            "\neffort: \"high\"\n",
            "\nbackground: true\n",
        ] {
            assert!(worker.contains(line), "{line}: {worker}");
        }
        assert!(!worker.contains("\nisolation:"), "{worker}");
        assert!(
            worker.contains("\ndescription: \"One piece of work in one project"),
            "{worker}"
        );
        assert!(
            worker.ends_with(&format!("\n---\n{FORK_PROMPT}")),
            "{worker}"
        );
        assert!(!worker.contains("\nprompt:"), "{worker}");
    }

    #[test]
    fn the_agent_files_omit_the_unset_fork_model_and_effort() {
        let worker = &agent_files(&ForkAgent::default())[1].1;
        assert!(!worker.contains("\nmodel:"), "{worker}");
        assert!(!worker.contains("\neffort:"), "{worker}");
    }

    #[test]
    fn the_agent_files_and_the_agents_json_come_from_the_same_definitions() {
        let fork = tuned_fork();
        let agents: Value = serde_json::from_str(&agents_json(&fork)).unwrap();
        for (file, contents) in agent_files(&fork) {
            let name = file.strip_suffix(".md").unwrap();
            let fields = agents[name].as_object().unwrap();
            for (key, value) in fields.iter().filter(|(key, _)| *key != "prompt") {
                assert!(
                    contents.contains(&format!("\n{key}: {value}\n")),
                    "{key}: {contents}"
                );
            }
        }
    }

    #[test]
    fn writing_the_agent_files_creates_both_and_replaces_stale_contents() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude").join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("project-fork.md"), "stale").unwrap();
        write_agent_files(dir.path(), &tuned_fork()).unwrap();
        for (file, contents) in agent_files(&tuned_fork()) {
            assert_eq!(
                std::fs::read_to_string(agents.join(file)).unwrap(),
                contents
            );
        }
        write_agent_files(dir.path(), &ForkAgent::default()).unwrap();
        let worker = std::fs::read_to_string(agents.join("project-fork.md")).unwrap();
        assert!(!worker.contains("claude-opus-5-5"), "{worker}");
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
        assert!(!FORK_PROMPT.contains("isolated"), "{FORK_PROMPT}");
    }

    #[test]
    fn the_settings_carry_the_cache_ttl_and_every_hook() {
        let program = Path::new("/plugins/it's/qol-cli-sessions");
        let settings: Value = serde_json::from_str(&settings_json(program)).unwrap();
        assert_eq!(settings["subagentPromptCacheTtl"], "1h");
        let hooks = &settings["hooks"];
        let command_of = |entry: &Value| entry["hooks"][0]["command"].as_str().unwrap().to_owned();
        let prefix = "'/plugins/it'\\''s/qol-cli-sessions' sessions switchboard hook";
        assert_eq!(settings["subagentStatusLine"]["type"], "command");
        assert_eq!(
            settings["subagentStatusLine"]["command"],
            format!("{prefix} agent-rows")
        );
        assert_eq!(
            hooks.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["PreToolUse", "SubagentStart"]
        );
        assert_eq!(hooks["PreToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(hooks["PreToolUse"][0]["matcher"], "Agent");
        assert_eq!(
            command_of(&hooks["PreToolUse"][0]),
            format!("{prefix} pre-agent")
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
