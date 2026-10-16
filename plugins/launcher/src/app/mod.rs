use std::path::PathBuf;
use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::DaemonRequest;

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

const FLOW_ACTION: &str = "flow";

pub enum Command {
    Show,
    Flow(String),
    Reload,
    Settings,
    Kill,
    Theme {
        native: Option<String>,
        accent: Option<String>,
    },
}

pub fn send_show() -> bool {
    core_daemon::send_action(&DAEMON_CONFIG, "show", true)
}

pub fn send_flow(plugin_id: &str) -> bool {
    core_daemon::send_action(&DAEMON_CONFIG, &format!("flow {plugin_id}"), true)
}

pub fn send_kill() -> bool {
    core_daemon::send_kill(&DAEMON_CONFIG)
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    core_daemon::start_request_listener(&DAEMON_CONFIG, tx, parse_request)
}

pub fn cleanup() {
    core_daemon::cleanup(&DAEMON_CONFIG);
}

pub(crate) fn socket_path() -> Option<PathBuf> {
    core_daemon::socket_path(&DAEMON_CONFIG)
}

fn parse_request(request: &DaemonRequest) -> ReadResult<Command> {
    if request.action != FLOW_ACTION {
        return parse_command(&request.action);
    }
    match flow_plugin(&request.input) {
        Some(plugin_id) => ReadResult::Command(Command::Flow(plugin_id)),
        None => ReadResult::Error("the flow action needs input.plugin".to_string()),
    }
}

fn flow_plugin(input: &serde_json::Value) -> Option<String> {
    input
        .get("plugin")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|plugin_id| !plugin_id.is_empty())
        .map(str::to_string)
}

fn parse_command(cmd: &str) -> ReadResult<Command> {
    #[cfg(debug_assertions)]
    if cmd != "ping" {
        qol_runtime::probe!("CMD_RECV", "cmd={cmd}");
    }
    if cmd == "theme" || cmd.starts_with("theme ") {
        let rest = &cmd[5..];
        let mut tokens = rest.split_whitespace().take(2);
        let native = tokens.next().filter(|t| *t != "-").map(str::to_string);
        let accent = tokens.next().filter(|t| *t != "-").map(str::to_string);
        return ReadResult::Command(Command::Theme { native, accent });
    }
    if let Some(plugin_id) = cmd
        .strip_prefix("flow ")
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        return ReadResult::Command(Command::Flow(plugin_id.to_string()));
    }
    match cmd {
        "ping" => ReadResult::Handled,
        "show" | "open" => ReadResult::Command(Command::Show),
        "reload" => ReadResult::Command(Command::Reload),
        "settings" => ReadResult::Command(Command::Settings),
        "kill" => ReadResult::Command(Command::Kill),
        _ => ReadResult::Fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_command, parse_request, Command};
    use qol_plugin_daemon::daemon::ReadResult;
    use qol_runtime::protocol::DaemonRequest;

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

    #[test]
    fn parses_flow_with_its_plugin_id() {
        match parse_command("flow qol-clipboard") {
            ReadResult::Command(Command::Flow(plugin_id)) => assert_eq!(plugin_id, "qol-clipboard"),
            _ => panic!("expected Flow command"),
        }
        assert!(matches!(parse_command("flow "), ReadResult::Fallback));
    }

    fn request(action: &str, input: serde_json::Value) -> DaemonRequest {
        DaemonRequest {
            action: action.to_string(),
            input,
        }
    }

    #[test]
    fn the_flow_action_names_its_plugin_in_the_input() {
        match parse_request(&request(
            "flow",
            serde_json::json!({ "plugin": "qol-clipboard" }),
        )) {
            ReadResult::Command(Command::Flow(plugin_id)) => assert_eq!(plugin_id, "qol-clipboard"),
            _ => panic!("expected Flow command"),
        }
        for input in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({ "plugin": "  " }),
            serde_json::json!({ "plugin": 7 }),
        ] {
            assert!(
                matches!(
                    parse_request(&request("flow", input.clone())),
                    ReadResult::Error(_)
                ),
                "{input}"
            );
        }
    }

    #[test]
    fn string_commands_still_arrive_as_bare_actions() {
        let null = serde_json::Value::Null;
        match parse_request(&request("flow qol-memory", null.clone())) {
            ReadResult::Command(Command::Flow(plugin_id)) => assert_eq!(plugin_id, "qol-memory"),
            _ => panic!("expected Flow command"),
        }
        assert!(matches!(
            parse_request(&request("show", null.clone())),
            ReadResult::Command(Command::Show)
        ));
        assert!(matches!(
            parse_request(&request("ping", null.clone())),
            ReadResult::Handled
        ));
        assert!(matches!(
            parse_request(&request("nope", null)),
            ReadResult::Fallback
        ));
    }
}
