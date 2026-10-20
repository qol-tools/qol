use std::fmt::{Display, Formatter};

use super::CliToolId;

/// A model or effort the tool cannot take, worded for the caller who asked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchFlagError(String);

impl Display for LaunchFlagError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for LaunchFlagError {}

type Result<T> = std::result::Result<T, LaunchFlagError>;

pub const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub fn launch_flags(
    tool: &CliToolId,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<Vec<String>> {
    let mut flags = permission_flags(tool);
    flags.extend(tier_flags(tool, model, effort)?);
    Ok(flags)
}

pub fn tier_flags(
    tool: &CliToolId,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<Vec<String>> {
    let mut flags = Vec::new();
    if let Some(model) = model {
        flags.extend(model_flags(tool, model)?);
    }
    if let Some(effort) = effort {
        flags.extend(effort_flags(tool, effort)?);
    }
    Ok(flags)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaneProfile {
    pub settings_json: Option<String>,
    pub mcp_config_json: Option<String>,
}

pub fn lane_profile_flags(tool: &CliToolId, profile: &LaneProfile) -> Vec<String> {
    if tool.as_str() != "claude" {
        return Vec::new();
    }
    let mut flags = Vec::new();
    if let Some(settings) = &profile.settings_json {
        flags.extend(["--settings".to_owned(), settings.clone()]);
    }
    if let Some(mcp_config) = &profile.mcp_config_json {
        flags.extend([
            "--strict-mcp-config".to_owned(),
            format!("--mcp-config={mcp_config}"),
        ]);
    }
    flags
}

pub fn remote_control_flags(tool: &CliToolId, name: &str) -> Result<Vec<String>> {
    match tool.as_str() {
        "claude" => Ok(vec!["--remote-control".to_owned(), name.to_owned()]),
        other => Err(LaunchFlagError(format!(
            "tool `{other}` has no remote control launch flag; drop remote_control or pick a tool that takes one"
        ))),
    }
}

pub fn takes_remote_control(tool: &str) -> bool {
    tool == "claude"
}

fn permission_flags(tool: &CliToolId) -> Vec<String> {
    match tool.as_str() {
        "claude" => vec!["--dangerously-skip-permissions".to_owned()],
        _ => Vec::new(),
    }
}

pub const RESUMABLE_PERMISSION_MODES: [&str; 5] = [
    "default",
    "acceptEdits",
    "plan",
    "auto",
    "bypassPermissions",
];

pub fn resume_permission_flags(tool: &CliToolId, mode: Option<&str>) -> Vec<String> {
    match (tool.as_str(), mode) {
        ("claude", Some("bypassPermissions")) => permission_flags(tool),
        ("claude", Some(mode)) if RESUMABLE_PERMISSION_MODES.contains(&mode) => {
            vec!["--permission-mode".to_owned(), mode.to_owned()]
        }
        _ => Vec::new(),
    }
}

fn model_flags(tool: &CliToolId, model: &str) -> Result<Vec<String>> {
    let flag = match tool.as_str() {
        "pi" | "codex" | "claude" | "kimi" => "--model",
        other => {
            return Err(LaunchFlagError(format!(
            "tool `{other}` has no model override flag; launch it directly with the model instead",
        )))
        }
    };
    Ok(vec![flag.to_owned(), model.to_owned()])
}

pub fn takes_effort(tool: &str) -> bool {
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
        return Err(LaunchFlagError(format!(
            "invalid effort `{effort}`; expected one of {}",
            EFFORT_LEVELS.join(", ")
        )));
    }
    let Some(flag) = effort_flag(tool.as_str()) else {
        return Err(LaunchFlagError(format!(
            "tool `{}` has no effort flag; drop the effort or pick a tool that takes one",
            tool.as_str()
        )));
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
    fn resumes_keep_the_recorded_permission_mode_and_never_widen_an_unknown_one() {
        let cases: [(&str, Option<&str>, &[&str]); 6] = [
            (
                "claude",
                Some("bypassPermissions"),
                &["--dangerously-skip-permissions"],
            ),
            ("claude", Some("plan"), &["--permission-mode", "plan"]),
            ("claude", Some("default"), &["--permission-mode", "default"]),
            ("claude", Some("yolo"), &[]),
            ("claude", None, &[]),
            ("pi", Some("bypassPermissions"), &[]),
        ];
        for (id, mode, expected) in cases {
            assert_eq!(
                resume_permission_flags(&tool(id), mode),
                expected,
                "{id} {mode:?}"
            );
        }
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
    fn a_claude_lane_profile_layers_settings_and_pins_its_mcp_servers() {
        let profile = LaneProfile {
            settings_json: Some(r#"{"enabledPlugins":{"heavy@market":false}}"#.to_owned()),
            mcp_config_json: Some(r#"{"mcpServers":{}}"#.to_owned()),
        };
        assert_eq!(
            lane_profile_flags(&tool("claude"), &profile),
            [
                "--settings",
                r#"{"enabledPlugins":{"heavy@market":false}}"#,
                "--strict-mcp-config",
                r#"--mcp-config={"mcpServers":{}}"#,
            ]
        );
    }

    #[test]
    fn the_mcp_config_rides_in_its_flag_so_the_variadic_list_cannot_swallow_the_prompt() {
        let profile = LaneProfile {
            settings_json: None,
            mcp_config_json: Some(r#"{"mcpServers":{}}"#.to_owned()),
        };
        let flags = lane_profile_flags(&tool("claude"), &profile);
        assert!(!flags.contains(&"--mcp-config".to_owned()), "{flags:?}");
        assert!(flags
            .last()
            .is_some_and(|flag| flag.starts_with("--mcp-config=")));
    }

    #[test]
    fn lane_profiles_only_reach_claude_and_an_empty_profile_adds_nothing() {
        let profile = LaneProfile {
            settings_json: Some("{}".to_owned()),
            mcp_config_json: Some(r#"{"mcpServers":{}}"#.to_owned()),
        };
        for id in ["pi", "codex", "kimi"] {
            assert!(lane_profile_flags(&tool(id), &profile).is_empty(), "{id}");
        }
        assert!(lane_profile_flags(&tool("claude"), &LaneProfile::default()).is_empty());
    }

    #[test]
    fn remote_control_names_the_claude_session_and_other_tools_refuse_it() {
        assert_eq!(
            remote_control_flags(&tool("claude"), "chase-lockfile").unwrap(),
            ["--remote-control", "chase-lockfile"]
        );
        assert!(takes_remote_control("claude"));
        for id in ["pi", "codex", "kimi"] {
            let error = remote_control_flags(&tool(id), "k")
                .unwrap_err()
                .to_string();
            assert!(error.contains("no remote control launch flag"), "{error}");
            assert!(!takes_remote_control(id), "{id}");
        }
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
