use std::io::Read;

use serde::Deserialize;

use plugin_cli_sessions::attention::{reduce, Attention, Evidence};
use plugin_cli_sessions::host::{kitty_session_id, Pane};
use plugin_cli_sessions::registry::summary_for;
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
        spawn_identity: None,
    };

    let interpreter = CliSessionInterpreter::system();
    let cli_session = interpreter.describe(&pane);
    let tool = Tool::from_cli_session(&cli_session);
    let screen_evidence = interpreter.classify_screen(&pane, &frame.screen);
    let evidence = Evidence {
        descriptor_runtime: cli_session.evidence.runtime,
        screen_runtime: screen_evidence.runtime,
        viewport: screen_evidence.viewport,
        file_fresh: cli_session.evidence.activity.file_fresh,
        screen_changed: true,
        at_prompt: frame.at_prompt,
        is_generic: tool == Tool::Generic,
        is_service: false,
    };
    let reduction = reduce(&Attention::default(), &evidence, 0);
    let status = reduction.attention.status;
    println!(
        "tool={:?} phase={:?} status={:?} summary={:?} label={:?}",
        tool,
        reduction.phase,
        status,
        summary_for(status, tool),
        cli_session.display_name,
    );
}
