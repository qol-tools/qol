use anyhow::{bail, Result};
use qol_terminal_sessions::cli::CliToolId;

pub(super) const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub(super) fn launch_flags(
    tool: &CliToolId,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<Vec<String>> {
    let mut flags = permission_flags(tool);
    if let Some(model) = model {
        flags.extend(model_flags(tool, model)?);
    }
    if let Some(effort) = effort {
        flags.extend(effort_flags(tool, effort)?);
    }
    Ok(flags)
}

fn permission_flags(tool: &CliToolId) -> Vec<String> {
    match tool.as_str() {
        "claude" => vec!["--dangerously-skip-permissions".to_owned()],
        _ => Vec::new(),
    }
}

fn model_flags(tool: &CliToolId, model: &str) -> Result<Vec<String>> {
    let flag = match tool.as_str() {
        "pi" | "codex" | "claude" | "kimi" => "--model",
        other => bail!(
            "tool `{other}` has no model override flag; launch it directly with the model instead"
        ),
    };
    Ok(vec![flag.to_owned(), model.to_owned()])
}

pub(super) fn takes_effort(tool: &str) -> bool {
    effort_flag(tool).is_some()
}

fn effort_flag(tool: &str) -> Option<&'static str> {
    match tool {
        "claude" => Some("--effort"),
        "pi" => Some("--thinking"),
        _ => None,
    }
}

fn effort_flags(tool: &CliToolId, effort: &str) -> Result<Vec<String>> {
    if !EFFORT_LEVELS.contains(&effort) {
        bail!(
            "invalid effort `{effort}`; expected one of {}",
            EFFORT_LEVELS.join(", ")
        );
    }
    let Some(flag) = effort_flag(tool.as_str()) else {
        bail!(
            "tool `{}` has no effort flag; drop the effort or pick a tool that takes one",
            tool.as_str()
        );
    };
    Ok(vec![flag.to_owned(), effort.to_owned()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(id: &str) -> CliToolId {
        CliToolId::new(id).unwrap()
    }

    #[test]
    fn claude_launches_skip_permission_prompts_and_carry_model_and_effort() {
        assert_eq!(
            launch_flags(&tool("claude"), Some("sonnet-x"), Some("medium")).unwrap(),
            [
                "--dangerously-skip-permissions",
                "--model",
                "sonnet-x",
                "--effort",
                "medium"
            ]
        );
    }

    #[test]
    fn pi_takes_its_effort_as_a_thinking_level_and_gets_no_permission_flag() {
        assert_eq!(
            launch_flags(&tool("pi"), Some("flash-x"), Some("high")).unwrap(),
            ["--model", "flash-x", "--thinking", "high"]
        );
        assert!(launch_flags(&tool("pi"), None, None).unwrap().is_empty());
    }

    #[test]
    fn every_registered_tool_takes_a_model_and_unknown_tools_are_refused() {
        for id in ["pi", "codex", "claude", "kimi"] {
            let flags = launch_flags(&tool(id), Some("flash-x"), None).unwrap();
            assert!(flags.ends_with(&["--model".to_owned(), "flash-x".to_owned()]));
        }
        let error = launch_flags(&tool("generic"), Some("flash-x"), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no model override flag"), "{error}");
    }

    #[test]
    fn effort_is_validated_and_refused_for_tools_without_an_effort_flag() {
        let error = launch_flags(&tool("claude"), None, Some("colossal"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid effort"), "{error}");
        let error = launch_flags(&tool("codex"), None, Some("high"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("no effort flag"), "{error}");
    }
}
