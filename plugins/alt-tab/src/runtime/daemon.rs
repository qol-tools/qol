use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

pub(crate) const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: false,
};

pub enum Command {
    Show,
    ShowReverse,
    Reload,
    Settings,
    Kill,
    Theme {
        native: Option<String>,
        accent: Option<String>,
    },
}

pub fn send_show() -> bool {
    core_daemon::send_action(&CONFIG, "show", false)
}

pub fn send_show_reverse() -> bool {
    core_daemon::send_action(&CONFIG, "show-reverse", false)
}

pub fn send_kill() -> bool {
    core_daemon::send_kill(&CONFIG)
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    core_daemon::start_listener(&CONFIG, tx, parse_command)
}

pub fn cleanup() {
    core_daemon::cleanup(&CONFIG);
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
        "show-reverse" | "open-reverse" => ReadResult::Command(Command::ShowReverse),
        "reload" => ReadResult::Command(Command::Reload),
        "settings" => ReadResult::Command(Command::Settings),
        "kill" => ReadResult::Command(Command::Kill),
        _ => ReadResult::Fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_parser_preserves_every_runtime_route() {
        let cases = [
            ("ping", "handled"),
            ("show", "show"),
            ("open", "show"),
            ("show-reverse", "show-reverse"),
            ("open-reverse", "show-reverse"),
            ("reload", "reload"),
            ("settings", "settings"),
            ("kill", "kill"),
            ("unknown", "fallback"),
        ];

        for (input, expected) in cases {
            let actual = match parse_command(input) {
                ReadResult::Handled => "handled",
                ReadResult::Command(Command::Show) => "show",
                ReadResult::Command(Command::ShowReverse) => "show-reverse",
                ReadResult::Command(Command::Reload) => "reload",
                ReadResult::Command(Command::Settings) => "settings",
                ReadResult::Command(Command::Kill) => "kill",
                ReadResult::Command(Command::Theme { .. }) => "theme",
                ReadResult::Fallback => "fallback",
                ReadResult::HandledWithData(_) => "handled-with-data",
                ReadResult::Error(_) => "error",
                ReadResult::Ignore => "ignore",
            };

            assert_eq!(actual, expected, "input={input}");
        }
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
