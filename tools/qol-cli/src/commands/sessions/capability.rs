use std::ffi::OsString;
use std::process::Command;

use anyhow::{anyhow, bail, Result};
use qol_terminal_sessions::cli::{CliModelCatalog, CliSessionInterpreter, CliToolId};
use serde::Serialize;

use super::spawn::config_spawn_model;

const USAGE: &str = "qol sessions capability [--tier TOKEN]";

#[derive(Serialize)]
struct ToolCapability {
    tool: String,
    program: String,
    installed: bool,
    models: Vec<String>,
    tier_models: Vec<String>,
}

#[derive(Serialize)]
struct Capability {
    tier: Option<String>,
    lane_spawn: bool,
    spawn_model: Option<String>,
    tools: Vec<ToolCapability>,
}

fn parse_args(args: &[OsString]) -> Result<Option<String>> {
    let mut tier = None;
    let mut index = 0;
    while index < args.len() {
        let value = args[index]
            .to_str()
            .ok_or_else(|| anyhow!("capability flags must be UTF-8\nusage: {USAGE}"))?;
        match value {
            "--tier" => {
                let token = args
                    .get(index + 1)
                    .and_then(|token| token.to_str())
                    .filter(|token| !token.starts_with("--") && !token.trim().is_empty())
                    .ok_or_else(|| anyhow!("--tier requires a value\nusage: {USAGE}"))?;
                tier = Some(token.trim().to_lowercase());
                index += 2;
            }
            other => bail!("unknown capability flag `{other}`\nusage: {USAGE}"),
        }
    }
    Ok(tier)
}

fn installed(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
}

fn catalog_models(catalog: &CliModelCatalog) -> Vec<String> {
    let Ok(output) = Command::new(&catalog.program).args(&catalog.args).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip(catalog.header_rows)
        .filter_map(|line| line.split_whitespace().nth(catalog.model_column))
        .map(str::to_owned)
        .collect()
}

fn tool_capability(
    interpreter: &CliSessionInterpreter,
    tool: &CliToolId,
    tier: Option<&str>,
) -> ToolCapability {
    let program = interpreter
        .launch_for(tool)
        .map(|launch| launch.program)
        .unwrap_or_default();
    let installed = installed(&program);
    let models = match interpreter.model_catalog_for(tool) {
        Some(catalog) if installed => catalog_models(&catalog),
        _ => Vec::new(),
    };
    let tier_models = match tier {
        Some(tier) => models
            .iter()
            .filter(|model| model.to_lowercase().contains(tier))
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    ToolCapability {
        tool: tool.to_string(),
        program,
        installed,
        models,
        tier_models,
    }
}

fn lane_spawn(tools: &[ToolCapability], tier: Option<&str>, spawn_model: Option<&str>) -> bool {
    let configured = spawn_model.is_some_and(|model| match tier {
        Some(tier) => model.to_lowercase().contains(tier),
        None => !model.trim().is_empty(),
    });
    if configured {
        return true;
    }
    tools.iter().any(|tool| {
        tool.installed
            && match tier {
                Some(_) => !tool.tier_models.is_empty(),
                None => true,
            }
    })
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    let tier = parse_args(args)?;
    let interpreter = CliSessionInterpreter::system();
    let spawn_model = config_spawn_model()?;
    let tools = interpreter
        .launchable_tools()
        .iter()
        .map(|tool| tool_capability(&interpreter, tool, tier.as_deref()))
        .collect::<Vec<_>>();
    let capability = Capability {
        lane_spawn: lane_spawn(&tools, tier.as_deref(), spawn_model.as_deref()),
        tier,
        spawn_model,
        tools,
    };
    println!("{}", serde_json::to_string(&capability)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(installed: bool, tier_models: &[&str]) -> ToolCapability {
        ToolCapability {
            tool: "example".to_owned(),
            program: "example".to_owned(),
            installed,
            models: Vec::new(),
            tier_models: tier_models
                .iter()
                .map(|model| (*model).to_owned())
                .collect(),
        }
    }

    #[test]
    fn a_tier_request_needs_a_matching_model_from_an_installed_tool() {
        assert!(lane_spawn(
            &[tool(true, &["fast-flash"])],
            Some("flash"),
            None
        ));
        assert!(!lane_spawn(&[tool(true, &[])], Some("flash"), None));
        assert!(!lane_spawn(
            &[tool(false, &["fast-flash"])],
            Some("flash"),
            None
        ));
    }

    #[test]
    fn a_configured_spawn_model_at_the_requested_tier_is_enough() {
        assert!(lane_spawn(
            &[tool(false, &[])],
            Some("flash"),
            Some("x-flash")
        ));
        assert!(!lane_spawn(
            &[tool(false, &[])],
            Some("flash"),
            Some("x-pro")
        ));
        assert!(lane_spawn(&[tool(false, &[])], None, Some("x-pro")));
        assert!(!lane_spawn(&[tool(false, &[])], None, Some("   ")));
    }

    #[test]
    fn an_installed_tool_alone_answers_an_untiered_question() {
        assert!(lane_spawn(&[tool(true, &[])], None, None));
        assert!(!lane_spawn(&[tool(false, &[])], None, None));
        assert!(!lane_spawn(&[], None, None));
    }

    #[test]
    fn tier_parsing_lowercases_and_rejects_a_missing_value() {
        assert_eq!(
            parse_args(&["--tier".into(), "Flash".into()]).unwrap(),
            Some("flash".to_owned())
        );
        assert_eq!(parse_args(&[]).unwrap(), None);
        assert!(parse_args(&["--tier".into()]).is_err());
        assert!(parse_args(&["--tier".into(), "--json".into()]).is_err());
        assert!(parse_args(&["--unknown".into()]).is_err());
    }
}
