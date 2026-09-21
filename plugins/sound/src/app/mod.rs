//! The daemon, and the only process that answers qol-tray.
//!
//! Every action the manifest declares is answered here: the runtime command
//! and the daemon are the same binary, so the tray never falls back to a
//! second process.

use std::process::ExitCode;
use std::sync::mpsc;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

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
    NextOutput,
    Volume,
    SetVolume,
    VolumeUp,
    VolumeDown,
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
            Command::Kill => break,
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
        "next_output" => Action::NextOutput,
        "volume" => Action::Volume,
        "set_volume" => Action::SetVolume,
        "volume_up" => Action::VolumeUp,
        "volume_down" => Action::VolumeDown,
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
        Action::NextOutput => next_output(),
        Action::Volume => volume(),
        Action::SetVolume => set_volume(input),
        Action::VolumeUp => volume_step(crate::volume::STEP),
        Action::VolumeDown => volume_step(-crate::volume::STEP),
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
    if device == SYSTEM_DEFAULT {
        return;
    }
    match crate::output::list() {
        Ok(rows) if rows.iter().any(|row| row.value == device) => {}
        Ok(_) => return,
        Err(error) => {
            eprintln!("sound: cannot list the sound outputs: {error:#}");
            return;
        }
    }
    if let Err(error) = crate::output::switch(&device) {
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
    if output == SYSTEM_DEFAULT {
        return ReadResult::Handled;
    }
    match crate::output::switch(output) {
        Ok(()) => ReadResult::Handled,
        Err(error) => ReadResult::Error(format!("{error:#}")),
    }
}

fn next_output() -> ReadResult<Command> {
    match crate::output::next() {
        Ok(_) => ReadResult::Handled,
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

fn volume_step(delta: i32) -> ReadResult<Command> {
    match crate::volume::step(delta) {
        Ok(_) => ReadResult::Handled,
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
            ("next_output", Action::NextOutput),
            ("volume", Action::Volume),
            ("set_volume", Action::SetVolume),
            ("volume_up", Action::VolumeUp),
            ("volume_down", Action::VolumeDown),
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
        let cases = [
            "sound",
            "switch_all",
            "Switch",
            "not json",
            "{",
            "{\"action\":",
            "action:",
            "[1,2,3]",
        ];
        for action in cases {
            match dispatch(action, &serde_json::Value::Null) {
                ReadResult::Error(message) => {
                    assert!(message.contains(action), "the error names the action");
                }
                _ => panic!("an unknown action must be an error, never a handled status"),
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
    fn switch_to_system_default_is_handled_without_touching_the_host() {
        match dispatch("switch", &serde_json::json!({ "output": "default" })) {
            ReadResult::Handled => {}
            _ => panic!("System Default must succeed without switching anything"),
        }
    }

    #[test]
    fn the_outputs_payload_is_the_row_array() {
        let rows = vec![OutputRow {
            value: "speaker-a".to_string(),
            label: "Luna 2".to_string(),
            picture: "speaker".to_string(),
            connected: true,
        }];
        let payload = outputs_payload(rows).expect("output rows must encode");
        assert!(payload.is_array(), "the outputs payload is a JSON array");
        assert_eq!(payload[0]["value"], "speaker-a");
        assert_eq!(payload[0]["label"], "Luna 2");
        assert_eq!(payload[0]["picture"], "speaker");
        assert_eq!(payload[0]["connected"].as_bool(), Some(true));
    }

    #[test]
    fn the_output_status_payload_is_the_status_object() {
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
            "the output_status payload is a JSON object"
        );
        assert_eq!(payload["state"], "released");
        for field in ["saved", "applied", "shown", "detail"] {
            assert!(
                payload.get(field).is_some(),
                "the status object keeps `{field}`"
            );
        }
    }

    const PROBE_CHILD_ENV: &str = "QOL_SOUND_BACKEND_PROBE_CHILD";
    const PROBE_CHILD_RAN: &str = "QOL_SOUND_BACKEND_PROBE_CHILD_RAN";

    #[test]
    fn the_live_queries_are_probed_against_an_isolated_backend() {
        let root =
            std::env::temp_dir().join(format!("qol-sound-backend-probe-{}", std::process::id()));
        let output = std::process::Command::new(std::env::current_exe().expect("the test binary"))
            .arg("--exact")
            .arg("--nocapture")
            .arg("app::tests::isolated_backend_probe_child")
            .env(PROBE_CHILD_ENV, "1")
            .env("PULSE_SERVER", "unix:/nonexistent-qol-sound-probe")
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("HOME", &root)
            .output()
            .expect("the isolated backend probe runs");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stdout.contains(PROBE_CHILD_RAN),
            "the isolated probe never ran: {stdout:?} {stderr:?}"
        );
        assert!(
            output.status.success(),
            "the isolated probe failed: {stdout:?} {stderr:?}"
        );
    }

    #[test]
    fn isolated_backend_probe_child() {
        if std::env::var_os(PROBE_CHILD_ENV).is_none() {
            return;
        }
        println!("{PROBE_CHILD_RAN}");
        match dispatch("reload", &serde_json::Value::Null) {
            ReadResult::Command(Command::Reload) => {}
            ReadResult::Error(message) => {
                panic!("the isolated data home holds no saved configuration: {message}")
            }
            _ => panic!("reload must acknowledge the generation or refuse with the reason"),
        }
        for action in ["outputs", "output_status"] {
            match dispatch(action, &serde_json::Value::Null) {
                ReadResult::Error(message) => {
                    assert!(
                        !message.trim().is_empty(),
                        "an unreachable backend must carry the real reason"
                    );
                }
                _ => panic!("`{action}` must refuse when the backend is unreachable"),
            }
        }
    }
}
