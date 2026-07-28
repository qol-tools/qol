use plugin_cli_sessions::host::{kitty_session_id, Pane};
use plugin_cli_sessions::tool::Tool;
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::SessionCapabilities;

#[test]
fn classify_picks_agent_from_process_group() {
    let cases = [
        (
            vec!["zsh".to_string(), "claude".to_string(), "node".to_string()],
            Tool::Claude,
        ),
        (vec!["zsh".to_string(), "codex".to_string()], Tool::Codex),
        (
            vec!["zsh".to_string(), "codex".to_string(), "claude".to_string()],
            Tool::Codex,
        ),
        (vec!["zsh".to_string(), "npm".to_string()], Tool::Generic),
        (vec!["zsh".to_string()], Tool::Generic),
        (vec![], Tool::Generic),
    ];
    for (names, want) in cases {
        let pane = Pane {
            id: kitty_session_id(1),
            root_pid: 1,
            cwd: "/work/project".to_owned(),
            title: "Terminal".to_owned(),
            at_prompt: false,
            reported_cmd: None,
            foreground_basenames: names.clone(),
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
        };
        let descriptor = CliSessionInterpreter::system().describe(&pane);
        assert_eq!(
            Tool::from_cli_session(&descriptor),
            want,
            "names: {names:?}"
        );
    }
}
