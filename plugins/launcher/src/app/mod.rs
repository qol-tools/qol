use std::path::PathBuf;
use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

pub enum Command {
    Show,
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

pub fn send_kill() -> bool {
    core_daemon::send_kill(&DAEMON_CONFIG)
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    core_daemon::start_listener(&DAEMON_CONFIG, tx, parse_command)
}

pub fn cleanup() {
    core_daemon::cleanup(&DAEMON_CONFIG);
}

pub(crate) fn socket_path() -> Option<PathBuf> {
    core_daemon::socket_path(&DAEMON_CONFIG)
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
    use super::{parse_command, Command};
    use qol_plugin_daemon::daemon::ReadResult;

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
