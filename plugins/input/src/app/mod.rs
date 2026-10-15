mod platform;

use std::process::ExitCode;
use std::sync::mpsc;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use serde::Serialize;

use crate::peripherals::{self, session, Level, Switch};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Ping,
    Kill,
    Reload,
    Theme,
    Mouse,
    Touchpad,
    Keyboard,
    Set(Level),
    Turn(Switch, bool),
    ToggleTouchpad,
    Settings,
}

enum Command {
    Kill,
}

pub fn run() -> ExitCode {
    session::recover();
    let (tx, rx) = mpsc::channel();
    let stop = tx.clone();
    platform::on_stop_request(move || drop(stop.send(Command::Kill)));
    if !core_daemon::start_request_listener(&DAEMON_CONFIG, tx, |request| {
        dispatch(&request.action, &request.input)
    }) {
        log::error!("failed to start the daemon listener");
        return ExitCode::FAILURE;
    }
    match rx.recv() {
        Ok(Command::Kill) | Err(mpsc::RecvError) => core_daemon::cleanup(&DAEMON_CONFIG),
    }
    session::release();
    ExitCode::SUCCESS
}

fn classify(action: &str) -> Option<Action> {
    let fixed = match action {
        "ping" => Some(Action::Ping),
        "kill" => Some(Action::Kill),
        "reload" => Some(Action::Reload),
        "mouse" => Some(Action::Mouse),
        "touchpad" => Some(Action::Touchpad),
        "keyboard" => Some(Action::Keyboard),
        "toggle_touchpad" => Some(Action::ToggleTouchpad),
        "settings" => Some(Action::Settings),
        theme if theme == "theme" || theme.starts_with("theme ") => Some(Action::Theme),
        _ => None,
    };
    fixed
        .or_else(|| level_action(action))
        .or_else(|| switch_action(action))
}

fn level_action(action: &str) -> Option<Action> {
    let key = action.strip_prefix("set_")?;
    Level::ALL
        .into_iter()
        .find(|level| level.key() == key)
        .map(Action::Set)
}

fn switch_action(action: &str) -> Option<Action> {
    let (key, on) = action
        .strip_suffix("_on")
        .map(|key| (key, true))
        .or_else(|| action.strip_suffix("_off").map(|key| (key, false)))?;
    Switch::ALL
        .into_iter()
        .find(|switch| switch.key() == key)
        .map(|switch| Action::Turn(switch, on))
}

fn dispatch(action: &str, input: &serde_json::Value) -> ReadResult<Command> {
    let Some(parsed) = classify(action) else {
        return ReadResult::Error(format!("unknown action `{action}`"));
    };
    match parsed {
        Action::Ping | Action::Reload | Action::Theme => ReadResult::Handled,
        Action::Kill => ReadResult::Command(Command::Kill),
        Action::Mouse => query(peripherals::mouse(), "failed to read the mouse settings"),
        Action::Touchpad => query(
            peripherals::touchpad(),
            "failed to read the touchpad settings",
        ),
        Action::Keyboard => query(
            peripherals::keyboard(),
            "failed to read the keyboard settings",
        ),
        Action::Set(level) => set_level(action, level, input),
        Action::Turn(switch, on) => handled(peripherals::set_switch(switch, on)),
        Action::ToggleTouchpad => handled(peripherals::toggle_touchpad().map(|_| ())),
        Action::Settings => settings(),
    }
}

fn set_level(action: &str, level: Level, input: &serde_json::Value) -> ReadResult<Command> {
    let Some(value) = requested_value(input) else {
        return ReadResult::Error(format!(
            "{action} requires a value from {}",
            level.accepted()
        ));
    };
    handled(peripherals::set_level(level, value))
}

fn requested_value(input: &serde_json::Value) -> Option<u32> {
    let value = input.get("value")?.as_f64()?.round();
    (0.0..=f64::from(u32::MAX))
        .contains(&value)
        .then_some(value as u32)
}

fn query<T: Serialize>(result: anyhow::Result<T>, failure: &str) -> ReadResult<Command> {
    match result {
        Ok(value) => match serde_json::to_value(value) {
            Ok(data) => ReadResult::HandledWithData(data),
            Err(error) => ReadResult::Error(format!("failed to encode the answer: {error}")),
        },
        Err(error) => ReadResult::Error(format!("{failure}: {error:#}")),
    }
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
        Err(error) => ReadResult::Error(format!("failed to open the Input settings: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn ping_reload_and_kill_are_answered_without_a_backend() {
        assert!(matches!(
            dispatch("ping", &serde_json::Value::Null),
            ReadResult::Handled
        ));
        assert!(matches!(
            dispatch("reload", &serde_json::Value::Null),
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
    fn every_setting_keeps_its_own_branches() {
        for level in Level::ALL {
            assert_eq!(
                classify(&format!("set_{}", level.key())),
                Some(Action::Set(level))
            );
        }
        for switch in Switch::ALL {
            assert_eq!(
                classify(&format!("{}_on", switch.key())),
                Some(Action::Turn(switch, true))
            );
            assert_eq!(
                classify(&format!("{}_off", switch.key())),
                Some(Action::Turn(switch, false))
            );
        }
        let cases = [
            ("ping", Action::Ping),
            ("kill", Action::Kill),
            ("reload", Action::Reload),
            ("mouse", Action::Mouse),
            ("touchpad", Action::Touchpad),
            ("keyboard", Action::Keyboard),
            ("toggle_touchpad", Action::ToggleTouchpad),
            ("settings", Action::Settings),
            ("theme", Action::Theme),
            ("theme bone amber", Action::Theme),
        ];
        for (name, expected) in cases {
            assert_eq!(classify(name), Some(expected), "`{name}` keeps its branch");
        }
    }

    #[test]
    fn an_unknown_action_is_an_error_that_names_it() {
        for action in [
            "input",
            "set_",
            "set_speed",
            "_on",
            "touchpad_toggle",
            "themes",
            "acceleration",
            "Mouse",
            "{\"action\":",
        ] {
            match dispatch(action, &serde_json::Value::Null) {
                ReadResult::Error(message) => {
                    assert!(message.contains(action), "the error names `{action}`");
                }
                _ => panic!("`{action}` must be an error, never a handled status"),
            }
        }
    }

    #[test]
    fn a_set_without_a_usable_value_is_an_error_that_names_the_action() {
        for level in Level::ALL {
            let action = format!("set_{}", level.key());
            for input in [
                serde_json::Value::Null,
                serde_json::json!({}),
                serde_json::json!({ "value": "fast" }),
                serde_json::json!({ "value": -5 }),
            ] {
                match dispatch(&action, &input) {
                    ReadResult::Error(message) => {
                        assert!(message.contains(&action), "the error names `{action}`");
                    }
                    _ => panic!("`{action}` without a value must be an error"),
                }
            }
        }
    }

    #[test]
    fn the_panel_value_is_read_whole_from_an_integer_or_a_float() {
        assert_eq!(
            requested_value(&serde_json::json!({ "value": 40 })),
            Some(40)
        );
        assert_eq!(
            requested_value(&serde_json::json!({ "value": 39.6 })),
            Some(40)
        );
        assert_eq!(requested_value(&serde_json::json!({ "value": -1 })), None);
        assert_eq!(requested_value(&serde_json::json!({ "value": 1e12 })), None);
    }

    #[test]
    fn the_contract_lays_out_mouse_touchpad_keyboard_and_validates_against_the_runtime() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let config =
            qol_config::contract::parse_spec("qol-config.toml").expect("qol-config.toml invalid");
        assert_eq!(manifest.plugin.name, "Input");
        assert_eq!(manifest.menu.label, "Input");
        assert_eq!(config.title.as_deref(), Some("Input"));
        let sections: Vec<&str> = config.sections.keys().map(String::as_str).collect();
        assert_eq!(sections, ["mouse", "touchpad", "keyboard"]);
        let runtime = qol_config::contract::parse_runtime_spec("qol-runtime.toml")
            .expect("qol-runtime.toml invalid");
        qol_config::contract::validate_contracts(&config, Some(&runtime))
            .expect("qol-config.toml and qol-runtime.toml must validate together");
    }

    #[test]
    fn each_level_slider_spans_its_bounds_and_lands_on_both_ends_this_computer_accepts() {
        let config =
            qol_config::contract::parse_spec("qol-config.toml").expect("qol-config.toml invalid");
        for level in Level::ALL {
            let number = &config.fields[level.key()].number;
            let (min, max, step) = (
                number.min.unwrap(),
                number.max.unwrap(),
                number.step.unwrap(),
            );
            let bounds = level.bounds();
            assert_eq!(min, f64::from(bounds.first), "{level:?}");
            assert_eq!(max, f64::from(bounds.last), "{level:?}");
            let values = level.values();
            for end in [values.first, values.last] {
                assert_eq!((f64::from(end) - min) % step, 0.0, "{level:?} {end}");
            }
        }
    }
}
