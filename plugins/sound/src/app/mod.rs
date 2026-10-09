//! The daemon, and the only process that answers qol-tray.
//!
//! Every action the manifest declares is answered here: the runtime command
//! and the daemon are the same binary, so the tray never falls back to a
//! second process.

use std::process::ExitCode;
use std::sync::{mpsc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use qol_audio::meter::Meter;
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use serde::Serialize;

use crate::config::SoundConfig;
use crate::device::{Role, INPUT, OUTPUT};
use crate::levels::Levels;
use crate::output::{OutputRow, OutputStatus, SYSTEM_DEFAULT};

const LEVELS_IDLE: Duration = Duration::from_secs(3);

struct Meters {
    output: Option<Meter>,
    input: Option<Meter>,
    last_ask: Instant,
    checked: Option<Instant>,
    output_percent: Option<u32>,
    input_percent: Option<u32>,
}

const METER_RECHECK: Duration = Duration::from_secs(1);

static METERS: Mutex<Option<Meters>> = Mutex::new(None);

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
    Inputs,
    InputStatus,
    SwitchInput,
    InputVolume,
    SetInputVolume,
    MuteStatus,
    MuteOutput,
    UnmuteOutput,
    MuteInput,
    UnmuteInput,
    Levels,
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
        log::error!("failed to start the daemon listener");
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
        "inputs" => Action::Inputs,
        "input_status" => Action::InputStatus,
        "switch_input" => Action::SwitchInput,
        "input_volume" => Action::InputVolume,
        "set_input_volume" => Action::SetInputVolume,
        "mute_status" => Action::MuteStatus,
        "mute_output" => Action::MuteOutput,
        "unmute_output" => Action::UnmuteOutput,
        "mute_input" => Action::MuteInput,
        "unmute_input" => Action::UnmuteInput,
        "levels" => Action::Levels,
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
        Action::Inputs => query(crate::input::list(), "failed to list microphones"),
        Action::InputStatus => query(
            crate::input::status(),
            "failed to read the microphone status",
        ),
        Action::SwitchInput => switch_input(input),
        Action::InputVolume => query(
            crate::volume::input_status(),
            "failed to read the microphone volume",
        ),
        Action::SetInputVolume => set_input_volume(input),
        Action::MuteStatus => query(crate::mute::status(), "failed to read the mute status"),
        Action::MuteOutput => handled(crate::mute::set(OUTPUT, true)),
        Action::UnmuteOutput => handled(crate::mute::set(OUTPUT, false)),
        Action::MuteInput => handled(crate::mute::set(INPUT, true)),
        Action::UnmuteInput => handled(crate::mute::set(INPUT, false)),
        Action::Levels => levels(),
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
            log::warn!("sound: cannot read the saved configuration: {error}");
            return;
        }
    };
    if inspection.source.is_none() {
        return;
    }
    if apply_saved_output(&inspection.config) {
        return;
    }
    apply_saved_input(&inspection.config.input.device);
}

fn apply_saved_output(config: &SoundConfig) -> bool {
    let device = config.output.device.as_str();
    if !saved_choice_connected(OUTPUT, device) {
        return false;
    }
    let before = match crate::device::effective(OUTPUT) {
        Ok(before) => before,
        Err(error) => {
            log::warn!("sound: {error:#}");
            None
        }
    };
    let applied = match crate::device::switch(OUTPUT, device) {
        Ok(applied) => applied,
        Err(error) => {
            log::warn!("sound: cannot apply the saved output `{device}`: {error:#}");
            return false;
        }
    };
    let before = before.as_ref().map(|identity| identity.as_str());
    if !should_follow(config.input.follow_output, before, applied.as_str()) {
        return false;
    }
    match crate::input::follow(&applied) {
        Ok(followed) => followed.is_some(),
        Err(error) => {
            log::warn!("sound: the microphone did not follow the output `{device}`: {error:#}");
            false
        }
    }
}

fn should_follow(follow_output: bool, before: Option<&str>, applied: &str) -> bool {
    follow_output && before != Some(applied)
}

fn apply_saved_input(device: &str) {
    if !saved_choice_connected(INPUT, device) {
        return;
    }
    if let Err(error) = crate::device::switch(INPUT, device) {
        log::warn!("sound: cannot apply the saved microphone `{device}`: {error:#}");
    }
}

fn saved_choice_connected(role: Role, device: &str) -> bool {
    if device == SYSTEM_DEFAULT {
        return false;
    }
    match crate::device::list(role) {
        Ok(rows) => rows.iter().any(|row| row.value == device),
        Err(error) => {
            log::warn!("sound: cannot list the {}: {error:#}", role.plural);
            false
        }
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
    let Some(output) = requested(input, "output") else {
        return ReadResult::Error("switch requires an output".to_string());
    };
    if output == SYSTEM_DEFAULT {
        return ReadResult::Handled;
    }
    handled(crate::output::switch(output))
}

fn switch_input(input: &serde_json::Value) -> ReadResult<Command> {
    let Some(microphone) = requested(input, "input") else {
        return ReadResult::Error("switch_input requires an input microphone".to_string());
    };
    if microphone == SYSTEM_DEFAULT {
        return ReadResult::Handled;
    }
    handled(crate::input::switch(microphone))
}

fn requested<'a>(input: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
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
    let Some(percent) = requested_percent(input) else {
        return ReadResult::Error(format!(
            "set_volume requires a value from 0 to {}",
            crate::volume::MAX_PERCENT
        ));
    };
    handled(crate::volume::set(percent))
}

fn set_input_volume(input: &serde_json::Value) -> ReadResult<Command> {
    let Some(percent) = requested_percent(input) else {
        return ReadResult::Error(format!(
            "set_input_volume requires a value from 0 to {}",
            crate::volume::MAX_PERCENT
        ));
    };
    handled(crate::volume::set_input(percent))
}

fn requested_percent(input: &serde_json::Value) -> Option<u32> {
    input
        .get("value")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn volume_step(delta: i32) -> ReadResult<Command> {
    match crate::volume::step(delta) {
        Ok(_) => ReadResult::Handled,
        Err(error) => ReadResult::Error(format!("{error:#}")),
    }
}

fn levels() -> ReadResult<Command> {
    match serde_json::to_value(read_levels(Instant::now())) {
        Ok(data) => ReadResult::HandledWithData(data),
        Err(error) => ReadResult::Error(format!("failed to encode the audio levels: {error}")),
    }
}

fn read_levels(now: Instant) -> Levels {
    let mut guard = METERS.lock().unwrap_or_else(PoisonError::into_inner);
    let opened = guard.is_none();
    let meters = guard.get_or_insert_with(|| Meters {
        output: None,
        input: None,
        last_ask: now,
        checked: None,
        output_percent: None,
        input_percent: None,
    });
    if meters
        .checked
        .is_none_or(|checked| now.duration_since(checked) >= METER_RECHECK)
    {
        meters.output = current_meter(meters.output.take(), OUTPUT);
        meters.input = current_meter(meters.input.take(), INPUT);
        meters.output_percent = crate::volume::percent().ok().flatten();
        meters.input_percent = crate::volume::input_percent().ok().flatten();
        meters.checked = Some(now);
    }
    meters.last_ask = now;
    let levels = Levels {
        output: crate::levels::before_volume(meters.output.as_ref(), meters.output_percent),
        input: crate::levels::before_volume(meters.input.as_ref(), meters.input_percent),
    };
    drop(guard);
    if opened {
        spawn_idle_close();
    }
    levels
}

fn current_meter(meter: Option<Meter>, role: Role) -> Option<Meter> {
    match meter {
        Some(meter) if crate::levels::is_current(&meter, role) => Some(meter),
        Some(_) | None => crate::levels::open(role),
    }
}

fn spawn_idle_close() {
    let spawned = std::thread::Builder::new()
        .name("qol-sound-levels".to_string())
        .spawn(close_meters_when_idle);
    if let Err(error) = spawned {
        log::warn!("sound: cannot watch the level meters for idleness, closing them: {error}");
        *METERS.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

fn close_meters_when_idle() {
    loop {
        let wait = {
            let mut guard = METERS.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(meters) = guard.as_ref() else {
                return;
            };
            match idle_wait(meters.last_ask, Instant::now()) {
                Some(wait) => wait,
                None => {
                    *guard = None;
                    return;
                }
            }
        };
        std::thread::sleep(wait);
    }
}

fn idle_wait(last_ask: Instant, now: Instant) -> Option<Duration> {
    LEVELS_IDLE
        .checked_sub(now.saturating_duration_since(last_ask))
        .filter(|wait| !wait.is_zero())
}

fn query<T: Serialize>(result: anyhow::Result<T>, failure: &str) -> ReadResult<Command> {
    match result {
        Ok(value) => match payload(&value) {
            Ok(data) => ReadResult::HandledWithData(data),
            Err(message) => ReadResult::Error(message),
        },
        Err(error) => ReadResult::Error(format!("{failure}: {error:#}")),
    }
}

fn payload<T: Serialize>(value: &T) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|error| format!("failed to encode the answer: {error}"))
}

fn handled(result: anyhow::Result<()>) -> ReadResult<Command> {
    match result {
        Ok(()) => ReadResult::Handled,
        Err(error) => ReadResult::Error(format!("{error:#}")),
    }
}

fn settings() -> ReadResult<Command> {
    match qol_apps::desktop_integration::open_plugin_settings_via_tray(crate::PLUGIN_ID) {
        Ok(()) => ReadResult::Handled,
        Err(error) => ReadResult::Error(format!("failed to open the Audio settings: {error}")),
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
            ("inputs", Action::Inputs),
            ("input_status", Action::InputStatus),
            ("switch_input", Action::SwitchInput),
            ("input_volume", Action::InputVolume),
            ("set_input_volume", Action::SetInputVolume),
            ("mute_status", Action::MuteStatus),
            ("mute_output", Action::MuteOutput),
            ("unmute_output", Action::UnmuteOutput),
            ("mute_input", Action::MuteInput),
            ("unmute_input", Action::UnmuteInput),
            ("levels", Action::Levels),
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
    fn the_contract_lays_out_output_then_input_and_validates_against_the_runtime() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let config =
            qol_config::contract::parse_spec("qol-config.toml").expect("qol-config.toml invalid");
        assert_eq!(manifest.plugin.name, "Audio");
        assert_eq!(manifest.menu.label, "Audio");
        assert_eq!(config.title.as_deref(), Some("Audio"));
        let sections: Vec<&str> = config.sections.keys().map(String::as_str).collect();
        assert_eq!(sections, ["output", "input"]);
        let rows: Vec<(&str, Option<&str>)> = config
            .fields
            .iter()
            .map(|(id, field)| (id.as_str(), field.section.as_deref()))
            .collect();
        assert_eq!(
            rows,
            [
                ("output_device", Some("output")),
                ("volume", Some("output")),
                ("output_mute", Some("output")),
                ("input_device", Some("input")),
                ("input_volume", Some("input")),
                ("input_mute", Some("input")),
                ("input_follow_output", Some("input")),
            ]
        );
        let runtime = qol_config::contract::parse_runtime_spec("qol-runtime.toml")
            .expect("qol-runtime.toml invalid");
        qol_config::contract::validate_contracts(&config, Some(&runtime))
            .expect("qol-config.toml and qol-runtime.toml must validate together");
    }

    #[test]
    fn switch_input_without_a_microphone_is_an_error() {
        for input in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({ "input": 7 }),
            serde_json::json!({ "input": "   " }),
            serde_json::json!({ "output": "speaker-a" }),
        ] {
            match dispatch("switch_input", &input) {
                ReadResult::Error(message) => {
                    assert!(
                        message.contains("microphone"),
                        "the error names the missing microphone"
                    );
                }
                _ => panic!("switch_input without a microphone must be an error"),
            }
        }
    }

    #[test]
    fn switch_input_to_system_default_is_handled_without_touching_the_host() {
        match dispatch("switch_input", &serde_json::json!({ "input": "default" })) {
            ReadResult::Handled => {}
            _ => panic!("System Default must succeed without switching anything"),
        }
    }

    #[test]
    fn a_volume_set_without_a_whole_percent_is_an_error() {
        for action in ["set_volume", "set_input_volume"] {
            for input in [
                serde_json::Value::Null,
                serde_json::json!({}),
                serde_json::json!({ "value": "loud" }),
                serde_json::json!({ "value": -5 }),
            ] {
                match dispatch(action, &input) {
                    ReadResult::Error(message) => {
                        assert!(message.contains(action), "the error names `{action}`");
                    }
                    _ => panic!("`{action}` without a percent must be an error"),
                }
            }
        }
    }

    #[test]
    fn the_microphone_follows_only_a_real_output_change() {
        let cases = [
            (true, Some("speaker-a"), "headset", true),
            (true, None, "headset", true),
            (true, Some("headset"), "headset", false),
            (false, Some("speaker-a"), "headset", false),
            (false, None, "headset", false),
        ];
        for (follow_output, before, applied, expected) in cases {
            assert_eq!(
                should_follow(follow_output, before, applied),
                expected,
                "follow {follow_output}, before {before:?}, applied {applied}"
            );
        }
    }

    #[test]
    fn the_level_meters_close_three_seconds_after_the_last_ask() {
        let asked = Instant::now();
        assert_eq!(idle_wait(asked, asked), Some(LEVELS_IDLE));
        assert_eq!(
            idle_wait(asked, asked + Duration::from_secs(1)),
            Some(Duration::from_secs(2))
        );
        assert_eq!(idle_wait(asked, asked + LEVELS_IDLE), None);
        assert_eq!(idle_wait(asked, asked + Duration::from_secs(10)), None);
    }

    #[test]
    fn a_query_answer_is_the_encoded_value_and_a_failure_names_the_query() {
        let rows = vec![crate::input::InputRow {
            value: "mic-a".to_string(),
            label: "Virtuoso".to_string(),
            picture: "mic".to_string(),
            connected: true,
        }];
        match query(Ok(rows), "failed to list microphones") {
            ReadResult::HandledWithData(data) => {
                assert!(data.is_array(), "the inputs payload is a JSON array");
                assert_eq!(data[0]["value"], "mic-a");
                assert_eq!(data[0]["label"], "Virtuoso");
            }
            _ => panic!("a listed answer must carry its data"),
        }
        match query::<()>(
            Err(anyhow::anyhow!("server gone")),
            "failed to list microphones",
        ) {
            ReadResult::Error(message) => {
                assert!(message.contains("failed to list microphones"));
                assert!(message.contains("server gone"));
            }
            _ => panic!("a failed query must be an error"),
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
        for action in [
            "outputs",
            "output_status",
            "inputs",
            "input_status",
            "mute_status",
        ] {
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
        match dispatch("levels", &serde_json::Value::Null) {
            ReadResult::HandledWithData(data) => {
                assert_eq!(data["output"].as_f64(), Some(0.0));
                assert_eq!(data["input"].as_f64(), Some(0.0));
            }
            _ => panic!("levels must read silence when nothing can be measured"),
        }
    }
}
