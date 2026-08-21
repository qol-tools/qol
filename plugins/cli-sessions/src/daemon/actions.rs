use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

pub const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

#[derive(Debug)]
pub enum Command {
    Open,
    NextAttention,
    Snapshot,
    Kill,
    Theme {
        native: Option<String>,
        accent: Option<String>,
    },
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
    core_daemon::start_listener(&CONFIG, tx, parse_command)
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
