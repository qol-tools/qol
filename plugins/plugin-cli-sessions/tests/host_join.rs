use plugin_cli_sessions::host::kitty::parse::parse_ls;

const SAMPLE: &str = r#"[{"id":1,"tabs":[{"id":1,"windows":[
{"id":10,"title":"✳ Improve logging","cwd":"/a/proj","pid":100,"at_prompt":false,"last_reported_cmdline":"claude","foreground_processes":[{"pid":100,"cmdline":["/bin/zsh"]},{"pid":101,"cmdline":["/usr/bin/claude"]}]},
{"id":11,"title":"qol-monorepo","cwd":"/a/cdx","pid":200,"at_prompt":false,"last_reported_cmdline":"codex","foreground_processes":[{"pid":201,"cmdline":["/x/codex"]}]},
{"id":12,"title":"~/proj","cwd":"/a/sh","pid":300,"at_prompt":true,"last_reported_cmdline":"","foreground_processes":[{"pid":300,"cmdline":["/bin/zsh"]}]}
]}]}]"#;

#[test]
fn parse_ls_extracts_panes_with_shell_integration() {
    let panes = parse_ls(SAMPLE).expect("parse").panes();
    assert_eq!(panes.len(), 3, "all panes returned");

    let claude = panes.iter().find(|p| p.window_id == 10).unwrap();
    assert_eq!(claude.root_pid, 100);
    assert_eq!(claude.reported_cmd.as_deref(), Some("claude"));
    assert!(!claude.at_prompt);
    assert!(claude.foreground_basenames.contains(&"claude".to_string()));
    assert_eq!(claude.foreground_pids, vec![100, 101]);

    let codex = panes.iter().find(|p| p.window_id == 11).unwrap();
    assert_eq!(codex.reported_cmd.as_deref(), Some("codex"));
    assert!(codex.foreground_basenames.contains(&"codex".to_string()));

    let shell = panes.iter().find(|p| p.window_id == 12).unwrap();
    assert!(shell.at_prompt, "bare shell is at_prompt");
    assert_eq!(shell.reported_cmd, None);
    assert!(shell.foreground_basenames.contains(&"zsh".to_string()));
}
