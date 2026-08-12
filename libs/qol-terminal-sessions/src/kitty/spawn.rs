use std::collections::HashMap;

use crate::cli::CliToolId;
use crate::{SpawnIdentity, SpawnKey, SpawnRequest, SpawnSurface, TerminalError};

const SESSION_KEY_TAG: &str = "qol_session_key";
const SESSION_TOOL_TAG: &str = "qol_session_tool";
const SESSION_SURFACE_TAG: &str = "qol_session_surface";

pub(super) fn surface_tag(surface: SpawnSurface) -> &'static str {
    match surface {
        SpawnSurface::Tab => "tab",
        SpawnSurface::OsWindow => "os_window",
    }
}

fn launch_type(surface: SpawnSurface) -> &'static str {
    match surface {
        SpawnSurface::Tab => "tab",
        SpawnSurface::OsWindow => "os-window",
    }
}

pub(super) fn surface_from_tag(tag: &str) -> Option<SpawnSurface> {
    match tag {
        "tab" => Some(SpawnSurface::Tab),
        "os_window" => Some(SpawnSurface::OsWindow),
        _ => None,
    }
}

pub(super) fn identity_from_user_vars(vars: &HashMap<String, String>) -> Option<SpawnIdentity> {
    let key = SpawnKey::new(vars.get(SESSION_KEY_TAG)?).ok()?;
    let tool = CliToolId::new(vars.get(SESSION_TOOL_TAG)?).ok()?;
    let surface = surface_from_tag(vars.get(SESSION_SURFACE_TAG)?)?;
    Some(SpawnIdentity { key, tool, surface })
}

pub(super) fn parse_spawned_window_id(stdout: &str) -> Option<u64> {
    let window_id = stdout.trim().parse::<u64>().ok()?;
    (window_id > 0).then_some(window_id)
}

pub(super) fn launch_argv(
    request: &SpawnRequest,
    path: Option<&str>,
    anchor_window_id: Option<u64>,
) -> Result<Vec<String>, TerminalError> {
    let mut argv = vec![
        "@".to_owned(),
        "launch".to_owned(),
        "--type".to_owned(),
        launch_type(request.identity.surface).to_owned(),
    ];
    if request.identity.surface == SpawnSurface::Tab {
        let anchor = anchor_window_id.ok_or_else(|| TerminalError::SpawnFailed {
            backend: super::backend_id().clone(),
            message: "cannot spawn a tab without the current window id".to_owned(),
        })?;
        argv.push("--next-to".to_owned());
        argv.push(format!("id:{anchor}"));
    }
    argv.push("--dont-take-focus".to_owned());
    if let Some(path) = path {
        argv.push("--env".to_owned());
        argv.push(format!("PATH={path}"));
    }
    for (key, value) in &request.launch.env {
        argv.push("--env".to_owned());
        argv.push(format!("{key}={value}"));
    }
    argv.push("--cwd".to_owned());
    argv.push(cwd_string(request)?);
    if let Some(title) = &request.title {
        argv.push("--title".to_owned());
        argv.push(title.clone());
    }
    argv.push("--var".to_owned());
    argv.push(format!("{SESSION_KEY_TAG}={}", request.identity.key));
    argv.push("--var".to_owned());
    argv.push(format!("{SESSION_TOOL_TAG}={}", request.identity.tool));
    argv.push("--var".to_owned());
    argv.push(format!(
        "{SESSION_SURFACE_TAG}={}",
        surface_tag(request.identity.surface)
    ));
    argv.push("--".to_owned());
    argv.push(request.launch.program.clone());
    argv.extend(request.launch.args.iter().cloned());
    Ok(argv)
}

fn cwd_string(request: &SpawnRequest) -> Result<String, TerminalError> {
    request
        .cwd
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| TerminalError::SpawnFailed {
            backend: super::backend_id().clone(),
            message: format!("cwd `{}` is not valid UTF-8", request.cwd.to_string_lossy()),
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::{
        identity_from_user_vars, launch_argv, parse_spawned_window_id, surface_from_tag,
        surface_tag, SESSION_KEY_TAG, SESSION_SURFACE_TAG, SESSION_TOOL_TAG,
    };
    use crate::cli::{CliLaunchProgram, CliToolId};
    use crate::{SpawnIdentity, SpawnKey, SpawnRequest, SpawnSurface, TerminalError};

    fn request(surface: SpawnSurface, title: Option<&str>) -> SpawnRequest {
        SpawnRequest {
            identity: SpawnIdentity {
                key: SpawnKey::new("voice-42").unwrap(),
                tool: CliToolId::new("codex").unwrap(),
                surface,
            },
            launch: CliLaunchProgram {
                program: "codex".to_owned(),
                args: vec!["--full-auto".to_owned(), "dir with spaces".to_owned()],
                env: Vec::new(),
            },
            cwd: "/work/project".into(),
            title: title.map(str::to_owned),
        }
    }

    fn vars(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn launch_argv_emits_the_configured_launch_env() {
        let mut request = request(SpawnSurface::Tab, None);
        request.launch.env = vec![("PI_MODEL".to_owned(), "deepseek-v4-flash".to_owned())];
        let argv = launch_argv(&request, None, Some(42)).unwrap();

        let env_pos = argv
            .iter()
            .position(|arg| arg == "--env")
            .expect("env pair present");
        assert_eq!(argv[env_pos + 1], "PI_MODEL=deepseek-v4-flash");
        let cwd_pos = argv
            .iter()
            .position(|arg| arg == "--cwd")
            .expect("cwd flag present");
        assert!(env_pos < cwd_pos, "env pairs precede the cwd flag");
    }

    #[test]
    fn launch_argv_contract_for_tab() {
        let argv = launch_argv(
            &request(SpawnSurface::Tab, Some("Codex")),
            Some("/usr/bin:/bin"),
            Some(42),
        )
        .unwrap();

        assert_eq!(
            argv,
            [
                "@",
                "launch",
                "--type",
                "tab",
                "--next-to",
                "id:42",
                "--dont-take-focus",
                "--env",
                "PATH=/usr/bin:/bin",
                "--cwd",
                "/work/project",
                "--title",
                "Codex",
                "--var",
                "qol_session_key=voice-42",
                "--var",
                "qol_session_tool=codex",
                "--var",
                "qol_session_surface=tab",
                "--",
                "codex",
                "--full-auto",
                "dir with spaces",
            ]
        );
    }

    #[test]
    fn launch_argv_contract_for_os_window() {
        let argv = launch_argv(&request(SpawnSurface::OsWindow, None), None, None).unwrap();

        assert_eq!(
            argv,
            [
                "@",
                "launch",
                "--type",
                "os-window",
                "--dont-take-focus",
                "--cwd",
                "/work/project",
                "--var",
                "qol_session_key=voice-42",
                "--var",
                "qol_session_tool=codex",
                "--var",
                "qol_session_surface=os_window",
                "--",
                "codex",
                "--full-auto",
                "dir with spaces",
            ]
        );
    }

    #[test]
    fn launch_argv_keeps_program_and_arguments_literal() {
        let mut request = request(SpawnSurface::OsWindow, None);
        request.launch.program = "prog".to_owned();
        request.launch.args = vec![
            "a b c".to_owned(),
            "--type".to_owned(),
            "tab".to_owned(),
            "x; rm -rf /tmp/canary-name".to_owned(),
            String::new(),
        ];

        let argv = launch_argv(&request, None, None).unwrap();

        assert_eq!(
            argv,
            [
                "@",
                "launch",
                "--type",
                "os-window",
                "--dont-take-focus",
                "--cwd",
                "/work/project",
                "--var",
                "qol_session_key=voice-42",
                "--var",
                "qol_session_tool=codex",
                "--var",
                "qol_session_surface=os_window",
                "--",
                "prog",
                "a b c",
                "--type",
                "tab",
                "x; rm -rf /tmp/canary-name",
                "",
            ]
        );
    }

    #[test]
    fn launch_argv_requires_an_anchor_only_for_tab() {
        let error = launch_argv(&request(SpawnSurface::Tab, None), None, None).unwrap_err();

        assert!(matches!(error, TerminalError::SpawnFailed { .. }));
        assert!(error.to_string().contains("current window"));
        assert!(launch_argv(&request(SpawnSurface::OsWindow, None), None, None).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn launch_argv_rejects_a_non_utf8_cwd() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let mut request = request(SpawnSurface::OsWindow, None);
        request.cwd = PathBuf::from(OsString::from_vec(vec![0xff]));

        let error = launch_argv(&request, None, None).unwrap_err();

        assert!(matches!(error, TerminalError::SpawnFailed { .. }));
        assert!(error.to_string().contains("UTF-8"));
    }

    #[test]
    fn parse_spawned_window_id_accepts_only_a_trimmed_positive_decimal() {
        let cases = [
            ("77\n", Some(77)),
            (" 77 \n", Some(77)),
            ("\n77", Some(77)),
            ("", None),
            ("   ", None),
            ("garbage", None),
            ("77 88", None),
            ("-1", None),
            ("0", None),
            ("0\n", None),
            ("77\n88", None),
            ("7a", None),
            ("18446744073709551616", None),
            ("99999999999999999999999999", None),
        ];
        for (stdout, expected) in cases {
            assert_eq!(
                parse_spawned_window_id(stdout),
                expected,
                "stdout: {stdout:?}"
            );
        }
    }

    #[test]
    fn surface_tags_round_trip_through_the_stamped_vocabulary() {
        for surface in [SpawnSurface::Tab, SpawnSurface::OsWindow] {
            assert_eq!(surface_from_tag(surface_tag(surface)), Some(surface));
        }
        for tag in ["", "tab ", "os-window", "window", "TAB", "os_window "] {
            assert_eq!(surface_from_tag(tag), None, "tag: {tag:?}");
        }
        assert_eq!(surface_tag(SpawnSurface::Tab), "tab");
        assert_eq!(surface_tag(SpawnSurface::OsWindow), "os_window");
    }

    #[test]
    fn identity_from_user_vars_parses_only_complete_valid_tags() {
        let complete = vars(&[
            (SESSION_KEY_TAG, "voice-42"),
            (SESSION_TOOL_TAG, "codex"),
            (SESSION_SURFACE_TAG, "tab"),
        ]);
        assert_eq!(
            identity_from_user_vars(&complete),
            Some(SpawnIdentity {
                key: SpawnKey::new("voice-42").unwrap(),
                tool: CliToolId::new("codex").unwrap(),
                surface: SpawnSurface::Tab,
            })
        );
        let os_window = vars(&[
            (SESSION_KEY_TAG, "lane-1"),
            (SESSION_TOOL_TAG, "pi"),
            (SESSION_SURFACE_TAG, "os_window"),
        ]);
        assert_eq!(
            identity_from_user_vars(&os_window).unwrap().surface,
            SpawnSurface::OsWindow
        );
        let with_extra = vars(&[
            (SESSION_KEY_TAG, "voice-42"),
            (SESSION_TOOL_TAG, "codex"),
            (SESSION_SURFACE_TAG, "tab"),
            ("unrelated_tag", "ignored"),
        ]);
        assert!(identity_from_user_vars(&with_extra).is_some());
    }

    #[test]
    fn identity_from_user_vars_rejects_partial_or_malformed_tags() {
        let cases = [
            vec![],
            vec![(SESSION_KEY_TAG, "voice-42")],
            vec![(SESSION_TOOL_TAG, "codex"), (SESSION_SURFACE_TAG, "tab")],
            vec![
                (SESSION_KEY_TAG, ""),
                (SESSION_TOOL_TAG, "codex"),
                (SESSION_SURFACE_TAG, "tab"),
            ],
            vec![
                (SESSION_KEY_TAG, "has space"),
                (SESSION_TOOL_TAG, "codex"),
                (SESSION_SURFACE_TAG, "tab"),
            ],
            vec![
                (SESSION_KEY_TAG, "voice-42"),
                (SESSION_TOOL_TAG, "has space"),
                (SESSION_SURFACE_TAG, "tab"),
            ],
            vec![
                (SESSION_KEY_TAG, "voice-42"),
                (SESSION_TOOL_TAG, "codex"),
                (SESSION_SURFACE_TAG, "banana"),
            ],
            vec![
                (SESSION_KEY_TAG, "voice-42"),
                (SESSION_TOOL_TAG, "codex"),
                (SESSION_SURFACE_TAG, ""),
            ],
        ];
        for entries in cases {
            let parsed = identity_from_user_vars(&vars(&entries));
            assert!(parsed.is_none(), "entries: {entries:?}");
        }
    }
}
