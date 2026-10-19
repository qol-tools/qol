use std::path::Path;

use anyhow::{anyhow, Result};
use qol_agent_launch::account::LaneExecCommand;
use qol_agent_launch::cap::SpawnCapConfig;
use qol_terminal_sessions::cli::{
    launch_flags, CliLaunchProgram, CliSessionInterpreter, CliToolId,
};

pub struct ResumePoint<'a> {
    pub tool: &'a CliToolId,
    pub external_id: &'a str,
    pub permission_mode: Option<&'a str>,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub claude_config_dir: Option<&'a Path>,
}

pub fn resume_launch(
    interpreter: &CliSessionInterpreter,
    point: &ResumePoint<'_>,
    cap: Option<&SpawnCapConfig>,
    lane_exec: &LaneExecCommand,
) -> Result<CliLaunchProgram> {
    let tool = point.tool;
    let mut launch = interpreter
        .launch_for(tool)
        .ok_or_else(|| anyhow!("`{tool}` has no launch program"))?;
    launch.args.extend(
        interpreter
            .resume_args_for(tool, point.external_id)
            .ok_or_else(|| anyhow!("`{tool}` has no resume command"))?,
    );
    launch.args.extend(launch_flags::resume_permission_flags(
        tool,
        point.permission_mode,
    ));
    launch
        .args
        .extend(launch_flags::tier_flags(tool, point.model, point.effort)?);
    let mut launch = qol_agent_launch::cap::wrap_launch(&launch, cap);
    qol_agent_launch::account::apply(&mut launch, tool, point.claude_config_dir, lane_exec)?;
    Ok(launch)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn lane_exec() -> LaneExecCommand {
        LaneExecCommand::current(&["sessions", "lane-exec"]).unwrap()
    }

    fn point<'a>(tool: &'a CliToolId, permission_mode: Option<&'a str>) -> ResumePoint<'a> {
        ResumePoint {
            tool,
            external_id: "abc",
            permission_mode,
            model: None,
            effort: None,
            claude_config_dir: None,
        }
    }

    #[test]
    fn a_resume_carries_its_permission_mode_tier_and_account_in_order() {
        let interpreter = CliSessionInterpreter::system();
        let tool = CliToolId::new("claude").unwrap();
        let home = PathBuf::from("/home/someone/.claude-work");
        let resumed = ResumePoint {
            model: Some("opus-x"),
            effort: Some("high"),
            claude_config_dir: Some(&home),
            ..point(&tool, Some("plan"))
        };
        let launch = resume_launch(&interpreter, &resumed, None, &lane_exec()).unwrap();
        assert_eq!(launch.program, "claude");
        assert_eq!(
            launch.args,
            [
                "--resume",
                "abc",
                "--permission-mode",
                "plan",
                "--model",
                "opus-x",
                "--effort",
                "high"
            ]
        );
        assert!(launch.env.contains(&(
            qol_agent_launch::account::CONFIG_DIR_ENV.to_owned(),
            "/home/someone/.claude-work".to_owned()
        )));
    }

    #[test]
    fn a_resume_keeps_its_args_after_the_scope_wrapper() {
        let interpreter = CliSessionInterpreter::system();
        let tool = CliToolId::new("claude").unwrap();
        let cap = SpawnCapConfig::default();
        let launch = resume_launch(
            &interpreter,
            &point(&tool, Some("bypassPermissions")),
            Some(&cap),
            &lane_exec(),
        )
        .unwrap();
        assert_eq!(launch.program, "systemd-run");
        let joined = launch.args.join(" ");
        assert!(
            joined.ends_with("-- claude --resume abc --dangerously-skip-permissions"),
            "{joined}"
        );
    }

    #[test]
    fn a_tool_without_a_resume_command_is_refused() {
        let interpreter = CliSessionInterpreter::system();
        let tool = CliToolId::new("no-such-tool").unwrap();
        assert!(resume_launch(&interpreter, &point(&tool, None), None, &lane_exec()).is_err());
    }
}
