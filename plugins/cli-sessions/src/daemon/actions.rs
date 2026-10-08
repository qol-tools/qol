use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::DaemonRequest;
use qol_terminal_sessions::SessionId;

use crate::ui::notify::FOCUS_ACTION;

pub const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

#[derive(Debug)]
pub enum Command {
    Open,
    NextAttention,
    Focus(SessionId),
    Snapshot,
    Kill,
    Theme {
        native: Option<String>,
        accent: Option<String>,
    },
}

fn parse_request(request: &DaemonRequest) -> ReadResult<Command> {
    if request.action != FOCUS_ACTION {
        return parse_command(&request.action);
    }
    match serde_json::from_value(request.input["session"].clone()) {
        Ok(session) => ReadResult::Command(Command::Focus(session)),
        Err(error) => ReadResult::Error(format!("focus requires a session: {error}")),
    }
}

fn parse_command(cmd: &str) -> ReadResult<Command> {
    if cmd == "theme" || cmd.starts_with("theme ") {
        let rest = &cmd[5..];
        let mut tokens = rest.split_whitespace().take(2);
        let native = tokens.next().filter(|t| *t != "-").map(str::to_string);
        let accent = tokens.next().filter(|t| *t != "-").map(str::to_string);
        return ReadResult::Command(Command::Theme { native, accent });
    }
    match cmd {
        "ping" => ReadResult::Handled,
        "open" => ReadResult::Command(Command::Open),
        "next" => ReadResult::Command(Command::NextAttention),
        "snapshot" => ReadResult::Command(Command::Snapshot),
        "kill" => ReadResult::Command(Command::Kill),
        _ => ReadResult::Fallback,
    }
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    core_daemon::start_request_listener(&CONFIG, tx, parse_request)
}

#[cfg(test)]
mod tests {
    use super::{parse_command, parse_request, Command};
    use qol_plugin_daemon::daemon::ReadResult;
    use qol_runtime::protocol::DaemonRequest;
    use qol_terminal_sessions::SessionId;

    #[test]
    fn focus_carries_the_session_from_the_notice_input() {
        let session = SessionId::new(qol_terminal_sessions::kitty::backend_id().clone(), "42")
            .expect("valid session id");
        let request = DaemonRequest {
            action: "focus".to_string(),
            input: serde_json::json!({ "session": session }),
        };
        match parse_request(&request) {
            ReadResult::Command(Command::Focus(parsed)) => assert_eq!(parsed, session),
            _ => panic!("expected Focus command"),
        }
    }

    #[test]
    fn focus_without_a_session_is_an_error() {
        let request = DaemonRequest {
            action: "focus".to_string(),
            input: serde_json::Value::Null,
        };
        assert!(matches!(parse_request(&request), ReadResult::Error(_)));
    }

    #[test]
    fn plain_actions_still_parse_through_the_request_listener() {
        let request = DaemonRequest {
            action: "next".to_string(),
            input: serde_json::Value::Null,
        };
        assert!(matches!(
            parse_request(&request),
            ReadResult::Command(Command::NextAttention)
        ));
    }

    fn theme(cmd: &str) -> (Option<String>, Option<String>) {
        match parse_command(cmd) {
            ReadResult::Command(Command::Theme { native, accent }) => (native, accent),
            _ => panic!("expected Theme command"),
        }
    }

    #[test]
    fn parses_theme_with_native_and_accent() {
        assert_eq!(
            theme("theme slate amber"),
            (Some("slate".to_string()), Some("amber".to_string()))
        );
    }

    #[test]
    fn parses_theme_with_dash_accent_as_none() {
        assert_eq!(theme("theme bone -"), (Some("bone".to_string()), None));
    }

    #[test]
    fn parses_bare_theme_as_all_none() {
        assert_eq!(theme("theme"), (None, None));
    }
}
