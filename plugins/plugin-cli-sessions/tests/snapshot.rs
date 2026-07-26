use std::collections::HashMap;
use std::fs;

use plugin_cli_sessions::host::{kitty_session_id, Pane, TerminalHost};
use plugin_cli_sessions::snapshot::capture_all;
use plugin_cli_sessions::status::Status;

struct FakeHost {
    panes: Vec<Pane>,
    screens: HashMap<u64, String>,
}

impl TerminalHost for FakeHost {
    fn discover(&self) -> Vec<Pane> {
        self.panes.clone()
    }
    fn get_text(&self, window_id: u64, _root_pid: i32) -> Option<String> {
        self.screens.get(&window_id).cloned()
    }
    fn focus(&self, _window_id: u64, _root_pid: i32) -> anyhow::Result<()> {
        Ok(())
    }
}

fn pane(window_id: u64, title: &str, fg: &[&str]) -> Pane {
    Pane {
        id: kitty_session_id(window_id),
        root_pid: 1,
        cwd: "/a/proj".into(),
        title: title.into(),
        at_prompt: false,
        reported_cmd: None,
        foreground_basenames: fg.iter().map(|s| s.to_string()).collect(),
        foreground_pids: vec![],
        capabilities: qol_terminal_sessions::SessionCapabilities::ALL,
    }
}

#[test]
fn snapshot_captures_every_window_with_its_panel_status() {
    let host = FakeHost {
        panes: vec![pane(1, "picker", &["claude"]), pane(2, "shell", &["zsh"])],
        screens: HashMap::from([
            (1, "\u{276F} 1. Yes\n  2. No".to_string()),
            (2, "$ ls".to_string()),
        ]),
    };
    // the panel currently (wrongly) shows the picker window as idle
    let panel = HashMap::from([(1u64, Status::Unknown), (2u64, Status::Working)]);

    let dir = std::env::temp_dir().join(format!("cli-sessions-snap-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);

    let target = capture_all(&host, &panel, &dir, 1234).expect("snapshot writes");

    let win1 = fs::read_to_string(target.join("win1.txt")).expect("win1 screen written");
    assert!(
        win1.contains("1. Yes"),
        "the real screen is captured verbatim"
    );

    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(target.join("win1.meta.json")).unwrap()).unwrap();
    assert_eq!(meta["title"], "picker");
    assert_eq!(meta["foreground_basenames"][0], "claude");
    assert_eq!(
        meta["expect"],
        serde_json::Value::Null,
        "left for the user to label"
    );
    assert_eq!(
        meta["panel_status"], "Unknown",
        "records what the panel showed, so a wrong status is visible against the screen"
    );

    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(target.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["windows"].as_array().unwrap().len(), 2);

    let _ = fs::remove_dir_all(&dir);
}
