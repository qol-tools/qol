use std::collections::HashMap;
use std::path::{Path, PathBuf};

use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::SessionId;

use crate::host::TerminalHost;
use crate::session::status::Status;
use crate::session::tool::from_cli_session;

pub fn capture_all(
    host: &dyn TerminalHost,
    panel: &HashMap<SessionId, Status>,
    dir: &Path,
    ts: u64,
) -> std::io::Result<PathBuf> {
    let target = dir.join(ts.to_string());
    std::fs::create_dir_all(&target)?;

    let panes = host.discover();
    let cli_interpreter = CliSessionInterpreter::system();
    let mut index = Vec::new();
    for pane in &panes {
        let cli_session = cli_interpreter.describe(pane);
        let screen = pane
            .binding()
            .ok()
            .and_then(|binding| host.get_text(&binding))
            .unwrap_or_default();
        let panel_status = panel.get(&pane.id).map(|s| format!("{s:?}"));
        let file_key = pane.id.native();
        let file = format!("session-{file_key}.txt");
        std::fs::write(target.join(&file), &screen)?;
        std::fs::write(
            target.join(format!("session-{file_key}.meta.json")),
            to_pretty(&serde_json::json!({
                "title": pane.title,
                "at_prompt": pane.at_prompt,
                "foreground_basenames": pane.foreground_basenames,
                "expect": serde_json::Value::Null,
                "panel_status": panel_status,
            })),
        )?;
        index.push(serde_json::json!({
            "session_id": pane.id,
            "file": file,
            "title": pane.title,
            "tool": from_cli_session(&cli_session).label,
            "cli_tool": cli_session.tool.id.as_str(),
            "panel_status": panel_status,
        }));
    }
    std::fs::write(
        target.join("report.json"),
        to_pretty(&serde_json::json!({ "ts": ts, "windows": index })),
    )?;
    Ok(target)
}

fn to_pretty(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec_pretty(value).unwrap_or_default()
}
