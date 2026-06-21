use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::host::TerminalHost;
use crate::status::Status;
use crate::tool::classify;

/// Dump every live session's frame in the moment - the screen, the title, and
/// the status the panel is currently showing - so a wrong status (in any
/// direction) can be turned into a 1:1 fixture. User-triggered, because only the
/// user knows the panel is wrong; the daemon has no oracle for its own misreads.
///
/// `panel` is the status the registry currently holds per window (what you see).
/// Each window is written in the corpus-fixture shape (`win<id>.txt` +
/// `win<id>.meta.json`) so a frame can be promoted to a regression test by
/// setting `expect`. Returns the snapshot directory.
pub fn capture_all(
    host: &dyn TerminalHost,
    panel: &HashMap<u64, Status>,
    dir: &Path,
    ts: u64,
) -> std::io::Result<PathBuf> {
    let target = dir.join(ts.to_string());
    std::fs::create_dir_all(&target)?;

    let panes = host.discover();
    let mut index = Vec::new();
    for pane in &panes {
        let screen = host.get_text(pane.window_id).unwrap_or_default();
        let panel_status = panel.get(&pane.window_id).map(|s| format!("{s:?}"));
        let file = format!("win{}.txt", pane.window_id);
        std::fs::write(target.join(&file), &screen)?;
        std::fs::write(
            target.join(format!("win{}.meta.json", pane.window_id)),
            to_pretty(&serde_json::json!({
                "title": pane.title,
                "at_prompt": pane.at_prompt,
                "foreground_basenames": pane.foreground_basenames,
                "expect": serde_json::Value::Null,
                "panel_status": panel_status,
            })),
        )?;
        index.push(serde_json::json!({
            "window_id": pane.window_id,
            "file": file,
            "title": pane.title,
            "tool": format!("{:?}", classify(&pane.foreground_basenames)),
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
