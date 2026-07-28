use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: false,
};

pub enum Command {
    Kill,
    Reload,
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
    parse_command_with(cmd, crate::theme::toggle, crate::theme::current)
}

fn parse_command_with<T, C>(cmd: &str, toggle: T, current: C) -> ReadResult<Command>
where
    T: FnOnce() -> anyhow::Result<crate::theme::ColorScheme>,
    C: FnOnce() -> anyhow::Result<crate::theme::ColorScheme>,
{
    match cmd {
        "ping" => ReadResult::Handled,
        "run" | "open" => ReadResult::Handled,
        "kill" => ReadResult::Command(Command::Kill),
        "reload" => ReadResult::Command(Command::Reload),
        "toggle_theme" => match toggle() {
            Ok(scheme) => {
                eprintln!("[os-themes] applied {scheme:?} theme");
                ReadResult::HandledWithData(theme_status(scheme))
            }
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        "theme_status" => match current() {
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
    use super::{parse_command_with, theme_status};
    use crate::theme::ColorScheme;
    use qol_plugin_daemon::daemon::ReadResult;

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

    #[test]
    fn toggle_acknowledges_only_after_returning_the_applied_state() {
        let result = parse_command_with(
            "toggle_theme",
            || Ok(ColorScheme::Dark),
            || panic!("toggle must not query current state separately"),
        );

        match result {
            ReadResult::HandledWithData(payload) => {
                assert_eq!(
                    payload,
                    serde_json::json!({ "scheme": "dark", "dark": true })
                );
            }
            ReadResult::Command(_)
            | ReadResult::Handled
            | ReadResult::Fallback
            | ReadResult::Error(_)
            | ReadResult::Ignore => {
                panic!("toggle must complete before the daemon acknowledges it")
            }
        }
    }

    #[test]
    fn toggle_failure_rejects_the_action() {
        let result = parse_command_with(
            "toggle_theme",
            || anyhow::bail!("theme backend unavailable"),
            || panic!("toggle failure must not query current state"),
        );

        match result {
            ReadResult::Error(message) => assert_eq!(message, "theme backend unavailable"),
            ReadResult::Command(_)
            | ReadResult::Handled
            | ReadResult::HandledWithData(_)
            | ReadResult::Fallback
            | ReadResult::Ignore => panic!("toggle failure must be returned to the caller"),
        }
    }
}
