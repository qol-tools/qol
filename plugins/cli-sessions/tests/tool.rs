use plugin_cli_sessions::host::{kitty_session_id, Pane};
use plugin_cli_sessions::tool::from_cli_session;
use qol_terminal_sessions::cli::{
    claude_tool, codex_tool, generic_tool, kimi_tool, pi_tool, CliSessionInterpreter,
};
use qol_terminal_sessions::SessionCapabilities;

#[test]
fn classify_picks_agent_from_process_group() {
    let cases = [
        (
            vec!["zsh".to_string(), "claude".to_string(), "node".to_string()],
            &claude_tool(),
        ),
        (vec!["zsh".to_string(), "codex".to_string()], &codex_tool()),
        (vec!["zsh".to_string(), "pi".to_string()], &pi_tool()),
        (
            vec!["zsh".to_string(), "kimi-code".to_string()],
            &kimi_tool(),
        ),
        (vec!["zsh".to_string(), "kimi".to_string()], &kimi_tool()),
        (
            vec!["zsh".to_string(), "codex".to_string(), "claude".to_string()],
            &codex_tool(),
        ),
        (vec!["zsh".to_string(), "npm".to_string()], &generic_tool()),
        (vec!["zsh".to_string()], &generic_tool()),
        (vec![], &generic_tool()),
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
            spawn_identity: None,
        };
        let descriptor = CliSessionInterpreter::system().describe(&pane);
        assert_eq!(from_cli_session(&descriptor), *want, "names: {names:?}");
    }
}
