use plugin_cli_sessions::tool::{classify, Tool};

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
        assert_eq!(classify(&names), want, "names: {names:?}");
    }
}
