//! The daemon, and the only process that answers qol-tray.
//!
//! Every action the manifest declares is answered here: the runtime command
//! and the daemon are the same binary, so the tray never falls back to a
//! second process.

use std::process::ExitCode;
use std::sync::mpsc;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

use crate::output::session::Released;
use crate::output::{OutputRow, OutputStatus, SYSTEM_DEFAULT};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Ping,
    Kill,
    Reload,
    Outputs,
    OutputStatus,
    Switch,
    GiveBack,
    Volume,
    SetVolume,
    Settings,
}

enum Command {
    Reload,
    Kill,
}

pub fn run() -> ExitCode {
    let (tx, rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&DAEMON_CONFIG, tx, |request| {
        dispatch(&request.action, &request.input)
    }) {
        eprintln!("sound: failed to start the daemon listener");
        return ExitCode::FAILURE;
    }
    while let Ok(command) = rx.recv() {
        match command {
            Command::Reload => apply_saved_choice(),
            Command::Kill => {
                if let Err(error) = crate::output::session::release(false) {
                    eprintln!("sound: failed to release the held output on stop: {error:#}");
                }
                break;
            }
        }
    }
    core_daemon::cleanup(&DAEMON_CONFIG);
    ExitCode::SUCCESS
}

fn classify(action: &str) -> Option<Action> {
    Some(match action {
        "ping" => Action::Ping,
        "kill" => Action::Kill,
        "reload" => Action::Reload,
        "outputs" => Action::Outputs,
        "output_status" => Action::OutputStatus,
        "switch" => Action::Switch,
        "give_back" => Action::GiveBack,
        "volume" => Action::Volume,
        "set_volume" => Action::SetVolume,
        "settings" => Action::Settings,
        _ => return None,
    })
}

fn dispatch(action: &str, input: &serde_json::Value) -> ReadResult<Command> {
    let Some(parsed) = classify(action) else {
        return ReadResult::Error(format!("unknown action `{action}`"));
    };
    match parsed {
        Action::Ping => ReadResult::Handled,
        Action::Kill => ReadResult::Command(Command::Kill),
        Action::Reload => reload(),
        Action::Outputs => outputs(),
        Action::OutputStatus => output_status(),
        Action::Switch => switch(input),
        Action::GiveBack => give_back(input),
        Action::Volume => volume(),
        Action::SetVolume => set_volume(input),
        Action::Settings => settings(),
    }
}

fn reload() -> ReadResult<Command> {
    match crate::config::inspect() {
        Ok(_) => ReadResult::Command(Command::Reload),
        Err(error) => ReadResult::Error(format!(
            "the saved sound configuration cannot be read: {error}"
        )),
    }
}

fn apply_saved_choice() {
    let inspection = match crate::config::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            eprintln!("sound: cannot read the saved configuration: {error}");
            return;
        }
    };
    if inspection.source.is_none() {
        return;
    }
    let device = inspection.config.output.device;
    let outcome = if device == SYSTEM_DEFAULT {
        crate::output::session::release(false).map(drop)
    } else {
        crate::output::session::claim(&device, false).map(drop)
    };
    if let Err(error) = outcome {
        eprintln!("sound: cannot apply the saved output `{device}`: {error:#}");
    }
}

fn outputs() -> ReadResult<Command> {
    match crate::output::list() {
        Ok(rows) => match outputs_payload(rows) {
            Ok(data) => ReadResult::HandledWithData(data),
            Err(message) => ReadResult::Error(message),
        },
        Err(error) => ReadResult::Error(format!("failed to list sound outputs: {error:#}")),
    }
}

fn outputs_payload(rows: Vec<OutputRow>) -> Result<serde_json::Value, String> {
    serde_json::to_value(rows).map_err(|error| format!("failed to encode sound outputs: {error}"))
}

fn output_status() -> ReadResult<Command> {
    match crate::output::status() {
        Ok(status) => match status_payload(&status) {
            Ok(data) => ReadResult::HandledWithData(data),
            Err(message) => ReadResult::Error(message),
        },
        Err(error) => {
            ReadResult::Error(format!("failed to read the sound output status: {error:#}"))
        }
    }
}

fn status_payload(status: &OutputStatus) -> Result<serde_json::Value, String> {
    serde_json::to_value(status)
        .map_err(|error| format!("failed to encode the sound output status: {error}"))
}

fn switch(input: &serde_json::Value) -> ReadResult<Command> {
    let Some(output) = input
        .get("output")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|output| !output.is_empty())
    else {
        return ReadResult::Error("switch requires an output".to_string());
    };
    let keep = input
        .get("keep")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let outcome = if output == SYSTEM_DEFAULT {
        crate::output::session::release(false).map(drop)
    } else {
        crate::output::session::claim(output, keep).map(drop)
    };
    match outcome {
        Ok(()) => ReadResult::Handled,
        Err(error) => ReadResult::Error(format!("{error:#}")),
    }
}

fn give_back(input: &serde_json::Value) -> ReadResult<Command> {
    let abandon = input
        .get("abandon")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    match crate::output::session::release(abandon) {
        Ok(Released::Output) => ReadResult::Handled,
        Ok(Released::Nothing) => ReadResult::Handled,
        Err(error) => ReadResult::Error(format!("{error:#}")),
    }
}

fn volume() -> ReadResult<Command> {
    match crate::volume::status() {
        Ok(status) => match serde_json::to_value(status) {
            Ok(data) => ReadResult::HandledWithData(data),
            Err(error) => ReadResult::Error(format!("failed to encode the sound volume: {error}")),
        },
        Err(error) => ReadResult::Error(format!("{error:#}")),
    }
}

fn set_volume(input: &serde_json::Value) -> ReadResult<Command> {
    let Some(percent) = input
        .get("value")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    else {
        return ReadResult::Error(format!(
            "set_volume requires a value from 0 to {}",
            crate::volume::MAX_PERCENT
        ));
    };
    match crate::volume::set(percent) {
        Ok(()) => ReadResult::Handled,
        Err(error) => ReadResult::Error(format!("{error:#}")),
    }
}

fn settings() -> ReadResult<Command> {
    match qol_apps::desktop_integration::open_plugin_settings_via_tray(crate::PLUGIN_ID) {
        Ok(()) => ReadResult::Handled,
        Err(error) => ReadResult::Error(format!("failed to open the Sound settings: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::StatusState;
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn ping_and_kill_are_answered_without_a_backend() {
        assert!(matches!(
            dispatch("ping", &serde_json::Value::Null),
            ReadResult::Handled
        ));
        assert!(matches!(
            dispatch("kill", &serde_json::Value::Null),
            ReadResult::Command(Command::Kill)
        ));
    }

    #[test]
    fn reload_accepts_the_saved_configuration_or_refuses_with_the_reason() {
        match dispatch("reload", &serde_json::Value::Null) {
            ReadResult::Command(Command::Reload) => {}
            ReadResult::Error(message) => {
                assert!(
                    !message.trim().is_empty(),
                    "a refused reload must carry the real reason"
                );
            }
            _ => panic!("reload must acknowledge the generation or refuse with the reason"),
        }
    }

    #[test]
    fn every_declared_action_and_query_maps_to_a_branch() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml must load");
        for id in manifest.executable_action_ids() {
            assert!(
                classify(&id).is_some(),
                "plugin.toml action `{id}` has no daemon branch"
            );
        }

        let runtime = qol_config::contract::parse_runtime_spec("qol-runtime.toml")
            .expect("qol-runtime.toml must load");
        for name in runtime.queries.keys() {
            assert!(
                classify(name).is_some(),
                "qol-runtime.toml query `{name}` has no daemon branch"
            );
        }
        let mapped = manifest.executable_action_ids();
        for name in runtime.actions.keys() {
            assert!(
                classify(name).is_some(),
                "qol-runtime.toml action `{name}` has no daemon branch"
            );
            assert!(
                mapped.iter().any(|id| id == name),
                "qol-runtime.toml action `{name}` has no plugin.toml action, so the tray refuses it"
            );
        }
    }

    #[test]
    fn the_declared_names_keep_their_branches() {
        let cases = [
            ("ping", Action::Ping),
            ("kill", Action::Kill),
            ("reload", Action::Reload),
            ("outputs", Action::Outputs),
            ("output_status", Action::OutputStatus),
            ("switch", Action::Switch),
            ("give_back", Action::GiveBack),
            ("volume", Action::Volume),
            ("set_volume", Action::SetVolume),
            ("settings", Action::Settings),
        ];
        for (name, expected) in cases {
            assert_eq!(
                classify(name),
                Some(expected),
                "`{name}` must keep its own branch"
            );
        }
    }

    #[test]
    fn an_unknown_action_is_an_error() {
        for action in ["sound", "give-back", "Switch"] {
            match dispatch(action, &serde_json::Value::Null) {
                ReadResult::Error(message) => {
                    assert!(message.contains(action), "the error names the action");
                }
                _ => panic!("an unknown action must be an error, never a handled status"),
            }
        }
    }

    #[test]
    fn a_malformed_request_line_is_an_error_and_does_not_panic() {
        for line in ["not json", "{", "{\"action\":", "action:", "[1,2,3]"] {
            match dispatch(line, &serde_json::Value::Null) {
                ReadResult::Error(_) => {}
                _ => panic!("a malformed request line must be an error"),
            }
        }
    }

    #[test]
    fn switch_without_an_output_is_an_error() {
        for input in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({ "output": 7 }),
            serde_json::json!({ "output": "   " }),
        ] {
            match dispatch("switch", &input) {
                ReadResult::Error(message) => {
                    assert!(
                        message.contains("output"),
                        "the error names the missing output"
                    );
                }
                _ => panic!("switch without an output must be an error"),
            }
        }
    }

    #[test]
    fn the_outputs_query_answers_with_a_json_array() {
        let rows = vec![OutputRow {
            value: "speaker-a".to_string(),
            label: "Luna 2".to_string(),
            picture: "speaker".to_string(),
            connected: true,
        }];
        let payload = outputs_payload(rows).expect("output rows must encode");
        assert!(
            payload.is_array(),
            "the outputs query answers with a JSON array"
        );
        assert_eq!(payload[0]["value"], "speaker-a");
        assert_eq!(payload[0]["label"], "Luna 2");
        assert_eq!(payload[0]["picture"], "speaker");
        assert_eq!(payload[0]["connected"].as_bool(), Some(true));

        match dispatch("outputs", &serde_json::Value::Null) {
            ReadResult::HandledWithData(data) => {
                assert!(data.is_array(), "the outputs branch keeps the array shape");
            }
            ReadResult::Error(message) => {
                assert!(
                    !message.trim().is_empty(),
                    "an unavailable listing must carry the real reason"
                );
            }
            _ => panic!("the outputs query must answer with rows or a real reason"),
        }
    }

    #[test]
    fn the_output_status_query_answers_with_the_status_object() {
        let status = OutputStatus {
            state: StatusState::Released,
            saved: None,
            applied: None,
            shown: SYSTEM_DEFAULT.to_string(),
            detail: None,
        };
        let payload = status_payload(&status).expect("the status must encode");
        assert!(
            payload.is_object(),
            "the output_status query answers with a JSON object"
        );
        assert_eq!(payload["state"], "released");
        for field in ["saved", "applied", "shown", "detail"] {
            assert!(
                payload.get(field).is_some(),
                "the status object keeps `{field}`"
            );
        }

        match dispatch("output_status", &serde_json::Value::Null) {
            ReadResult::HandledWithData(data) => {
                assert!(
                    data.is_object(),
                    "the output_status branch keeps the object shape"
                );
            }
            ReadResult::Error(message) => {
                assert!(
                    !message.trim().is_empty(),
                    "an unreadable status must carry the real reason"
                );
            }
            _ => panic!("the output_status query must answer with the status object or a reason"),
        }
    }
}
