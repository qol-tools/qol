use std::io::Read;

use serde::Deserialize;

use plugin_cli_sessions::host::Pane;
use plugin_cli_sessions::registry::summary_for;
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::strategy::codex::NoCodexStore;
use plugin_cli_sessions::strategy::{for_tool, status_for, Ctx};
use plugin_cli_sessions::tool::classify;

#[derive(Deserialize)]
struct Frame {
    #[serde(default)]
    title: String,
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
        window_id: 0,
        root_pid: 0,
        cwd: String::new(),
        title: frame.title,
        at_prompt: frame.at_prompt,
        reported_cmd: None,
        foreground_basenames: frame.foreground_basenames,
        foreground_pids: vec![],
    };

    let tool = classify(&pane.foreground_basenames);
    let strategy = for_tool(tool, &NoCodexStore);
    let reading = strategy.read(&Ctx {
        pane: &pane,
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
