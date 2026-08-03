use std::io::Read;

use serde::Deserialize;

use plugin_cli_sessions::host::{kitty_session_id, Pane};
use plugin_cli_sessions::registry::summary_for;
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::strategy::{for_tool, status_for, Ctx};
use plugin_cli_sessions::tool::Tool;
use qol_terminal_sessions::cli::CliSessionInterpreter;

#[derive(Deserialize)]
struct Frame {
    #[serde(default)]
    title: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    at_prompt: bool,
    #[serde(default)]
    foreground_basenames: Vec<String>,
    #[serde(default)]
    screen: String,
}

fn main() {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw).unwrap();
    let frame: Frame = serde_json::from_str(&raw).expect("stdin must be a Frame JSON object");

    let pane = Pane {
        id: kitty_session_id(0),
        root_pid: 0,
        cwd: frame.cwd,
        title: frame.title,
        at_prompt: frame.at_prompt,
        reported_cmd: None,
        foreground_basenames: frame.foreground_basenames,
        foreground_pids: vec![],
        capabilities: qol_terminal_sessions::SessionCapabilities::ALL,
    };

    let cli_session = CliSessionInterpreter::system().describe(&pane);
    let tool = Tool::from_cli_session(&cli_session);
    let strategy = for_tool(tool);
    let reading = strategy.read(&Ctx {
        pane: &pane,
        cli_session,
        screen: Some(&frame.screen),
        screen_changed: true,
        prev: None,
        now: 0,
        is_service: false,
    });
    let status = status_for(Status::Unknown, reading.phase);
    println!(
        "tool={:?} phase={:?} status={:?} summary={:?} label={:?}",
        tool,
        reading.phase,
        status,
        summary_for(status, tool),
        reading.label
    );
}
