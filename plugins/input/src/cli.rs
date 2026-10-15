use std::process::ExitCode;

use anyhow::{Context, Result};
use qol_headless::{Command, CommandContext, DoctorCheck, Execution, HeadlessApp};
use serde_json::Value;

use crate::peripherals::{self, Level, Setting, Status, Switch};
use crate::PLUGIN_ID;

const UNAVAILABLE: &str = "unavailable";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

type PlainHandler = Box<dyn Fn(&CommandContext) -> Result<Execution> + Send + Sync>;
type JsonHandler = Box<dyn Fn(&CommandContext) -> Result<Value> + Send + Sync>;

struct Handlers {
    status_plain: PlainHandler,
    status_json: JsonHandler,
    set: PlainHandler,
    toggle_touchpad: PlainHandler,
    settings: PlainHandler,
}

impl Handlers {
    fn live() -> Self {
        Self {
            status_plain: Box::new(status),
            status_json: Box::new(status_json),
            set: Box::new(set),
            toggle_touchpad: Box::new(toggle_touchpad),
            settings: Box::new(settings),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Change {
    Switch(Switch, bool),
    Level(Level, u32),
}

fn app() -> HeadlessApp {
    app_with_handlers(Handlers::live(), crate::doctor::checks())
}

fn app_with_handlers(handlers: Handlers, doctor_checks: Vec<DoctorCheck>) -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, PLUGIN_ID)
        .about("Set how the mouse, touchpad and keyboard respond.")
        .command(status_command(handlers.status_plain, handlers.status_json))
        .command(set_command(handlers.set))
        .command(toggle_touchpad_command(handlers.toggle_touchpad))
        .command(settings_command(handlers.settings))
        .doctor_checks(doctor_checks)
}

fn status_command(plain: PlainHandler, json: JsonHandler) -> Command {
    Command::new("status")
        .about("Show every mouse, touchpad and keyboard setting.")
        .usage(usage_status())
        .output(
            "One line per setting with the name to pass to set and its value, \
             unavailable where this computer has no such setting; a JSON object with --json.",
        )
        .exit_behavior("Exits 64 on a usage error; non-zero if the settings cannot be read.")
        .run_result(plain)
        .run_json(json)
}

fn set_command(handler: PlainHandler) -> Command {
    Command::new("set")
        .about("Change one setting.")
        .usage(usage_set())
        .detail(format!("Switches take on or off: {}.", switch_names()))
        .detail(format!("Levels take a whole number: {}.", level_ranges()))
        .output("One line with the setting and the value it took.")
        .exit_behavior(
            "Exits 64 on a usage error; non-zero if this computer cannot change the setting.",
        )
        .run_result(handler)
}

fn toggle_touchpad_command(handler: PlainHandler) -> Command {
    Command::new("toggle-touchpad")
        .about("Turn the touchpad off when it is on, and on when it is off.")
        .usage(usage_toggle_touchpad())
        .output("One line saying whether the touchpad is on.")
        .exit_behavior(
            "Exits 64 on a usage error; non-zero if there is no touchpad or it cannot be switched.",
        )
        .run_result(handler)
}

fn settings_command(handler: PlainHandler) -> Command {
    Command::new("settings")
        .about("Open the Input settings.")
        .usage(usage_settings())
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero if the settings cannot be opened.")
        .run_result(handler)
}

fn status(context: &CommandContext) -> Result<Execution> {
    if let Err(message) = parse_no_args(context.args(), usage_status()) {
        return Ok(Execution::usage(message));
    }
    let status = peripherals::status()?;
    let mut lines: Vec<String> = Setting::ALL
        .into_iter()
        .map(|setting| setting_line(setting, &status))
        .collect();
    if let Some(reason) = &status.touchpad.missing {
        lines.push(format!("the touchpad settings are unavailable: {reason}"));
    }
    Ok(Execution::success(newline_terminated(lines.join("\n"))))
}

fn status_json(context: &CommandContext) -> Result<Value> {
    parse_no_args(context.args(), usage_status()).map_err(anyhow::Error::msg)?;
    Ok(serde_json::to_value(peripherals::status()?)?)
}

fn set(context: &CommandContext) -> Result<Execution> {
    let change = match parse_set_args(context.args()) {
        Ok(change) => change,
        Err(message) => return Ok(Execution::usage(message)),
    };
    let setting = match change {
        Change::Switch(switch, on) => {
            peripherals::set_switch(switch, on)?;
            Setting::Switch(switch)
        }
        Change::Level(level, value) => {
            peripherals::set_level(level, value)?;
            Setting::Level(level)
        }
    };
    let status = peripherals::status()?;
    Ok(Execution::success(newline_terminated(setting_line(
        setting, &status,
    ))))
}

fn toggle_touchpad(context: &CommandContext) -> Result<Execution> {
    if let Err(message) = parse_no_args(context.args(), usage_toggle_touchpad()) {
        return Ok(Execution::usage(message));
    }
    peripherals::toggle_touchpad()?;
    let status = peripherals::status()?;
    Ok(Execution::success(newline_terminated(setting_line(
        Setting::Switch(Switch::Touchpad),
        &status,
    ))))
}

fn settings(context: &CommandContext) -> Result<Execution> {
    if let Err(message) = parse_no_args(context.args(), usage_settings()) {
        return Ok(Execution::usage(message));
    }
    qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID)
        .context("failed to open the Input settings")?;
    Ok(Execution::success(String::new()))
}

fn setting_line(setting: Setting, status: &Status) -> String {
    let value = match setting {
        Setting::Switch(switch) => match switch.read(status) {
            Some(true) => "on".to_string(),
            Some(false) => "off".to_string(),
            None => UNAVAILABLE.to_string(),
        },
        Setting::Level(level) => match level.read(status) {
            Some(value) => format!("{value}{}", level.unit()),
            None => UNAVAILABLE.to_string(),
        },
    };
    format!("{}  {value}", setting.name())
}

fn switch_names() -> String {
    Switch::ALL
        .into_iter()
        .map(|switch| Setting::Switch(switch).name())
        .collect::<Vec<_>>()
        .join(", ")
}

fn level_ranges() -> String {
    Level::ALL
        .into_iter()
        .map(|level| format!("{} {}", Setting::Level(level).name(), accepted_range(level)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn accepted_range(level: Level) -> String {
    let range = level.range();
    format!("{} to {}{}", range.start(), range.end(), level.unit())
}

fn usage_status() -> String {
    format!("{PLUGIN_ID} status [--json]")
}

fn usage_set() -> String {
    format!("{PLUGIN_ID} set <setting> <value>")
}

fn usage_toggle_touchpad() -> String {
    format!("{PLUGIN_ID} toggle-touchpad")
}

fn usage_settings() -> String {
    format!("{PLUGIN_ID} settings")
}

fn usage_error(usage: impl AsRef<str>, detail: impl AsRef<str>) -> String {
    format!("{}\n{}", detail.as_ref(), usage.as_ref())
}

fn unexpected(token: &str) -> String {
    if token.starts_with('-') {
        format!("unknown flag `{token}`")
    } else {
        format!("unexpected argument `{token}`")
    }
}

fn parse_no_args(args: &[String], usage: String) -> std::result::Result<(), String> {
    if let Some(token) = args.first() {
        return Err(usage_error(usage, unexpected(token)));
    }
    Ok(())
}

fn parse_set_args(args: &[String]) -> std::result::Result<Change, String> {
    let usage = usage_set();
    if let Some(flag) = args.iter().find(|token| token.starts_with('-')) {
        return Err(usage_error(&usage, unexpected(flag)));
    }
    let (name, value) = match args {
        [] => return Err(usage_error(&usage, "set requires a setting and a value")),
        [name] => (name, None),
        [name, value] => (name, Some(value)),
        [_, _, extra, ..] => return Err(usage_error(&usage, unexpected(extra))),
    };
    let Some(setting) = Setting::parse(name) else {
        let names: Vec<String> = Setting::ALL.into_iter().map(Setting::name).collect();
        return Err(usage_error(
            &usage,
            format!(
                "unknown setting `{name}`; choose one of {}",
                names.join(", ")
            ),
        ));
    };
    let accepted = match setting {
        Setting::Switch(_) => "on or off".to_string(),
        Setting::Level(level) => format!("a whole number from {}", accepted_range(level)),
    };
    let Some(value) = value else {
        return Err(usage_error(
            &usage,
            format!("set {name} requires {accepted}"),
        ));
    };
    let change = match setting {
        Setting::Switch(switch) => match value.as_str() {
            "on" => Some(Change::Switch(switch, true)),
            "off" => Some(Change::Switch(switch, false)),
            _ => None,
        },
        Setting::Level(level) => value
            .parse::<u32>()
            .ok()
            .filter(|value| level.range().contains(value))
            .map(|value| Change::Level(level, value)),
    };
    change.ok_or_else(|| usage_error(&usage, format!("{name} takes {accepted}, got `{value}`")))
}

fn newline_terminated(text: impl Into<String>) -> String {
    let mut text = text.into();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use qol_headless::{DoctorCheckResult, DoctorReport, DoctorStatus, EXIT_SUCCESS, EXIT_USAGE};
    use qol_plugin_api::manifest::PluginManifest;

    use super::*;
    use crate::peripherals::{Keyboard, Mouse, Touchpad};

    type Log = Arc<Mutex<Vec<String>>>;

    const COMMANDS: [&str; 4] = ["status", "set", "toggle-touchpad", "settings"];

    fn sentinel(log: &Log, name: &'static str) -> PlainHandler {
        let log = Arc::clone(log);
        Box::new(move |_context: &CommandContext| -> Result<Execution> {
            log.lock().unwrap().push(name.to_string());
            Ok(Execution::success(String::new()))
        })
    }

    fn json_sentinel(log: &Log, name: &'static str) -> JsonHandler {
        let log = Arc::clone(log);
        Box::new(move |_context: &CommandContext| -> Result<Value> {
            log.lock().unwrap().push(name.to_string());
            Ok(Value::Null)
        })
    }

    fn sentinel_app(log: &Log) -> HeadlessApp {
        app_with_doctor(log, test_check())
    }

    fn app_with_doctor(log: &Log, check: DoctorCheck) -> HeadlessApp {
        let handlers = Handlers {
            status_plain: sentinel(log, "status"),
            status_json: json_sentinel(log, "status"),
            set: sentinel(log, "set"),
            toggle_touchpad: sentinel(log, "toggle-touchpad"),
            settings: sentinel(log, "settings"),
        };
        app_with_handlers(handlers, vec![check])
    }

    fn test_check() -> DoctorCheck {
        DoctorCheck::new("test_check", "A deterministic test check.", || {
            Ok(DoctorCheckResult::ok("test_check", "ok"))
        })
    }

    fn args(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|token| token.to_string()).collect()
    }

    #[test]
    fn help_and_command_help_are_byte_identical_in_both_orders() {
        for command in COMMANDS.into_iter().chain(["doctor"]) {
            let log: Log = Arc::new(Mutex::new(Vec::new()));
            let app = sentinel_app(&log);
            let first = app.execute(args(&["help", command]));
            let final_token = app.execute(args(&[command, "help"]));

            assert_eq!(first.exit_code, EXIT_SUCCESS, "command: {command}");
            assert_eq!(first.stdout, final_token.stdout, "command: {command}");
            assert_eq!(first.stderr, final_token.stderr, "command: {command}");
            assert!(log.lock().unwrap().is_empty(), "command: {command}");
        }
    }

    #[test]
    fn set_help_lists_every_setting_with_what_it_takes() {
        let help = app().execute(args(&["help", "set"])).stdout;
        for setting in Setting::ALL {
            assert!(
                help.contains(&setting.name()),
                "{} missing from {help}",
                setting.name()
            );
        }
        assert!(help.contains("pointer-speed 0 to 100%"), "{help}");
        assert!(help.contains("repeat-rate 2 to 50 keys/s"), "{help}");
    }

    #[test]
    fn unknown_flags_exit_with_the_usage_code_and_name_the_usage() {
        let cases = [
            (vec!["status", "--bogus"], usage_status()),
            (vec!["set", "touchpad", "--bogus"], usage_set()),
            (vec!["toggle-touchpad", "--bogus"], usage_toggle_touchpad()),
            (vec!["settings", "--bogus"], usage_settings()),
        ];
        for (tokens, usage) in cases {
            let execution = app().execute(args(&tokens));
            assert_eq!(execution.exit_code, EXIT_USAGE, "args: {tokens:?}");
            assert!(execution.stdout.is_empty(), "args: {tokens:?}");
            assert!(
                execution.stderr.contains("unknown flag"),
                "args: {tokens:?}"
            );
            assert!(
                execution.stderr.contains(usage.as_str()),
                "args: {tokens:?}"
            );
        }
    }

    #[test]
    fn set_reads_a_switch_as_on_or_off_and_a_level_as_a_number_in_range() {
        assert_eq!(
            parse_set_args(&args(&["touchpad", "off"])),
            Ok(Change::Switch(Switch::Touchpad, false))
        );
        assert_eq!(
            parse_set_args(&args(&["tap_to_click", "on"])),
            Ok(Change::Switch(Switch::TapToClick, true))
        );
        assert_eq!(
            parse_set_args(&args(&["pointer-speed", "40"])),
            Ok(Change::Level(Level::PointerSpeed, 40))
        );
        assert_eq!(
            parse_set_args(&args(&["repeat-rate", "2"])),
            Ok(Change::Level(Level::RepeatRate, 2))
        );
    }

    #[test]
    fn set_refuses_what_it_cannot_apply_with_the_reason_and_the_usage() {
        let cases: [(&[&str], &str); 8] = [
            (&[], "set requires a setting and a value"),
            (
                &["pointer-speed"],
                "set pointer-speed requires a whole number from 0 to 100%",
            ),
            (&["touchpad"], "set touchpad requires on or off"),
            (&["speed", "40"], "unknown setting `speed`"),
            (&["touchpad", "yes"], "touchpad takes on or off, got `yes`"),
            (&["pointer-speed", "101"], "got `101`"),
            (&["repeat-delay", "fast"], "got `fast`"),
            (&["touchpad", "on", "now"], "unexpected argument `now`"),
        ];
        for (tokens, reason) in cases {
            let error = parse_set_args(&args(tokens)).unwrap_err();
            assert!(error.contains(reason), "{tokens:?}: {error}");
            assert!(error.contains(&usage_set()), "{tokens:?}: {error}");
        }
    }

    #[test]
    fn a_setting_line_names_the_setting_and_shows_its_value_or_that_it_is_unavailable() {
        let status = Status {
            backend: "Cinnamon",
            mouse: Mouse {
                speed: Some(40),
                acceleration: Some(false),
                swap_buttons: Some(true),
                double_click: Some(400),
                natural_scroll: None,
            },
            touchpad: Touchpad::default(),
            keyboard: Keyboard {
                repeat_delay: Some(500),
                repeat_rate: Some(33),
            },
        };
        let lines: Vec<String> = Setting::ALL
            .into_iter()
            .map(|setting| setting_line(setting, &status))
            .collect();
        assert_eq!(
            lines,
            [
                "pointer-speed  40%",
                "acceleration  off",
                "swap-buttons  on",
                "double-click  400 ms",
                "mouse-natural-scroll  unavailable",
                "touchpad  unavailable",
                "tap-to-click  unavailable",
                "touchpad-natural-scroll  unavailable",
                "touchpad-speed  unavailable",
                "repeat-delay  500 ms",
                "repeat-rate  33 keys/s",
            ]
        );
    }

    #[test]
    fn json_is_accepted_on_status_and_doctor_only() {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let app = sentinel_app(&log);
        let before = app.execute(args(&["--json", "status"]));
        let after = app.execute(args(&["status", "--json"]));
        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stdout, "null\n");

        let doctor = app.execute(args(&["doctor", "--json"]));
        let report: DoctorReport =
            serde_json::from_str(&doctor.stdout).expect("doctor output must be valid JSON");
        assert_eq!(report.plugin_id, PLUGIN_ID);
        assert_eq!(report.checks[0].id, "test_check");

        for tokens in [
            vec!["set", "touchpad", "on", "--json"],
            vec!["toggle-touchpad", "--json"],
            vec!["settings", "--json"],
        ] {
            let log: Log = Arc::new(Mutex::new(Vec::new()));
            let execution = sentinel_app(&log).execute(args(&tokens));
            assert_eq!(execution.exit_code, EXIT_USAGE, "args: {tokens:?}");
            assert!(
                execution.stderr.contains("does not support --json"),
                "args: {tokens:?}"
            );
            assert!(log.lock().unwrap().is_empty(), "args: {tokens:?}");
        }
    }

    #[test]
    fn doctor_exits_zero_for_a_valid_report_even_when_a_check_fails() {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let check = DoctorCheck::new("test_check", "A failing test check.", || {
            Ok(DoctorCheckResult::fail("test_check", "broken"))
        });
        let app = app_with_doctor(&log, check);
        let json = app.execute(args(&["doctor", "--json"]));
        assert_eq!(json.exit_code, EXIT_SUCCESS);
        let report: DoctorReport =
            serde_json::from_str(&json.stdout).expect("doctor output must be valid JSON");
        assert_eq!(report.status, DoctorStatus::Fail);
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn every_manifest_action_reaches_its_registered_handler() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let app = sentinel_app(&log);
        let mut expected = Vec::new();
        for action in manifest.executable_actions() {
            let argv = manifest
                .catalog_runtime_args(&action.id)
                .expect("executable action must have runtime args");
            let command = argv
                .first()
                .expect("runtime args must name a command")
                .clone();
            assert!(
                COMMANDS.contains(&command.as_str()),
                "`{command}` is not a command"
            );
            if let [_, setting, ..] = argv.as_slice() {
                assert!(
                    Setting::parse(setting).is_some(),
                    "action `{}` names the unknown setting `{setting}`",
                    action.id
                );
            }
            let execution = app.execute(argv);
            assert_eq!(execution.exit_code, EXIT_SUCCESS, "action: {}", action.id);
            expected.push(command);
        }
        expected.sort();
        expected.dedup();
        let mut recorded = log.lock().unwrap().clone();
        recorded.sort();
        recorded.dedup();
        assert_eq!(recorded, expected);
    }
}
