use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: false,
};

pub enum Command {
    Kill,
    Reload,
    ToggleTheme,
}

pub fn send_ping() -> bool {
    core_daemon::send_ping(&CONFIG)
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
    match cmd {
        "ping" => ReadResult::Handled,
        "run" | "open" => ReadResult::Handled,
        "kill" => ReadResult::Command(Command::Kill),
        "reload" => ReadResult::Command(Command::Reload),
        "toggle_theme" => ReadResult::Command(Command::ToggleTheme),
        "theme_status" => match crate::theme::current() {
            Ok(scheme) => ReadResult::HandledWithData(theme_status(scheme)),
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        _ => ReadResult::Fallback,
    }
}

fn theme_status(scheme: crate::theme::ColorScheme) -> serde_json::Value {
    serde_json::json!({
        "scheme": scheme.as_str(),
        "dark": scheme.is_dark(),
    })
}

#[cfg(test)]
mod tests {
    use super::theme_status;
    use crate::theme::ColorScheme;

    #[test]
    fn theme_status_payload_reports_semantic_and_binary_state() {
        let cases = [
            (
                ColorScheme::Light,
                serde_json::json!({ "scheme": "light", "dark": false }),
            ),
            (
                ColorScheme::Dark,
                serde_json::json!({ "scheme": "dark", "dark": true }),
            ),
        ];
        for (scheme, expected) in cases {
            assert_eq!(theme_status(scheme), expected);
        }
    }
}
