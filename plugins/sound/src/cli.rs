use std::process::ExitCode;

use anyhow::{Context, Result};
use qol_headless::{Command, CommandContext, DoctorCheck, Execution, HeadlessApp};
use serde_json::Value;

use crate::output::session::Released;
use crate::{BINARY_NAME, PLUGIN_ID};

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

type PlainHandler = Box<dyn Fn(&CommandContext) -> Result<Execution> + Send + Sync>;
type JsonHandler = Box<dyn Fn(&CommandContext) -> Result<Value> + Send + Sync>;

struct Handlers {
    outputs_plain: PlainHandler,
    outputs_json: JsonHandler,
    switch: PlainHandler,
    give_back: PlainHandler,
    volume: PlainHandler,
    settings: PlainHandler,
}

impl Handlers {
    fn live() -> Self {
        Self {
            outputs_plain: Box::new(outputs),
            outputs_json: Box::new(outputs_json),
            switch: Box::new(switch),
            give_back: Box::new(give_back),
            volume: Box::new(volume),
            settings: Box::new(settings),
        }
    }
}

fn app() -> HeadlessApp {
    app_with_handlers(Handlers::live(), crate::doctor::checks())
}

fn app_with_handlers(handlers: Handlers, doctor_checks: Vec<DoctorCheck>) -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Choose where sound plays, and give the output back when you are done.")
        .command(outputs_command(
            handlers.outputs_plain,
            handlers.outputs_json,
        ))
        .command(switch_command(handlers.switch))
        .command(give_back_command(handlers.give_back))
        .command(volume_command(handlers.volume))
        .command(settings_command(handlers.settings))
        .doctor_checks(doctor_checks)
}

fn outputs_command(plain: PlainHandler, json: JsonHandler) -> Command {
    Command::new("outputs")
        .about("List the sound outputs this computer can play on.")
        .usage(usage_outputs())
        .output(
            "One line per output with the value to pass to switch and its label; \
             a JSON array with --json.",
        )
        .exit_behavior("Exits non-zero if the outputs cannot be read.")
        .run_result(plain)
        .run_json(json)
}

fn switch_command(handler: PlainHandler) -> Command {
    Command::new("switch")
        .about("Make an output the one every app plays on.")
        .usage(usage_switch())
        .detail("Accepts an exact id or an unambiguous label match.")
        .detail("--keep records a durable choice, and only on a Resident host.")
        .output("One line when the output is under Sound's control.")
        .exit_behavior(
            "Exits 64 on a usage error; non-zero if the output is ambiguous, missing, \
             unavailable, or a durable choice is refused.",
        )
        .run_result(handler)
}

fn give_back_command(handler: PlainHandler) -> Command {
    Command::new("give-back")
        .about("Release the output Sound is holding.")
        .usage(usage_give_back())
        .detail("--abandon drops a record that cannot be restored, and says what remains.")
        .output("One line when the release completes.")
        .exit_behavior(
            "Exits 64 on a usage error; non-zero if the release fails or is still pending.",
        )
        .run_result(handler)
}

fn volume_command(handler: PlainHandler) -> Command {
    Command::new("volume")
        .about("Show or set the volume of the output everything plays on.")
        .usage(usage_volume())
        .detail("Without a percent, prints the current volume.")
        .output("One line with the volume in percent.")
        .exit_behavior("Exits 64 on a usage error; non-zero if the volume cannot be read or set.")
        .run_result(handler)
}

fn settings_command(handler: PlainHandler) -> Command {
    Command::new("settings")
        .about("Open the Sound settings.")
        .usage(usage_settings())
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero if the settings cannot be opened.")
        .run_result(handler)
}

fn outputs(context: &CommandContext) -> Result<Execution> {
    if let Err(message) = parse_outputs_args(context.args()) {
        return Ok(Execution::usage(message));
    }
    let rows = crate::output::list()?;
    let text = if rows.is_empty() {
        "no sound outputs found".to_string()
    } else {
        rows.iter().map(output_line).collect::<Vec<_>>().join("\n")
    };
    Ok(Execution::success(newline_terminated(text)))
}

fn outputs_json(context: &CommandContext) -> Result<Value> {
    parse_outputs_args(context.args()).map_err(anyhow::Error::msg)?;
    Ok(serde_json::to_value(crate::output::list()?)?)
}

fn output_line(row: &crate::output::OutputRow) -> String {
    if row.connected {
        return format!("{}  {}", row.value, row.label);
    }
    format!("{}  {}  (not connected)", row.value, row.label)
}

fn switch(context: &CommandContext) -> Result<Execution> {
    let (output, keep) = match parse_switch_args(context.args()) {
        Ok(parsed) => parsed,
        Err(message) => return Ok(Execution::usage(message)),
    };
    let _owner = crate::output::session::claim(&output, keep)?;
    Ok(Execution::success(newline_terminated(format!(
        "now controlling {output}"
    ))))
}

fn give_back(context: &CommandContext) -> Result<Execution> {
    let abandon = match parse_give_back_args(context.args()) {
        Ok(abandon) => abandon,
        Err(message) => return Ok(Execution::usage(message)),
    };
    give_back_with(abandon, crate::output::session::release)
}

fn give_back_with(
    abandon: bool,
    release: impl FnOnce(bool) -> Result<Released>,
) -> Result<Execution> {
    let released = release(abandon)?;
    let message = match (released, abandon) {
        (Released::Output, false) => "Sound no longer controls the output.",
        (Released::Output, true) => "Dropped Sound's output record.",
        (Released::Nothing, _) => "Nothing to give back: Sound was not holding the output.",
    };
    Ok(Execution::success(newline_terminated(message)))
}

fn volume(context: &CommandContext) -> Result<Execution> {
    let requested = match parse_volume_args(context.args()) {
        Ok(requested) => requested,
        Err(message) => return Ok(Execution::usage(message)),
    };
    if let Some(percent) = requested {
        crate::volume::set(percent)?;
    }
    let line = match crate::volume::percent()? {
        Some(percent) => format!("{percent}%"),
        None => "no sound output is playing".to_string(),
    };
    Ok(Execution::success(newline_terminated(line)))
}

fn settings(context: &CommandContext) -> Result<Execution> {
    if let Err(message) = parse_settings_args(context.args()) {
        return Ok(Execution::usage(message));
    }
    qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID)
        .context("failed to open the Sound settings")?;
    Ok(Execution::success(String::new()))
}

fn usage_outputs() -> String {
    format!("{BINARY_NAME} outputs [--json]")
}

fn usage_switch() -> String {
    format!("{BINARY_NAME} switch <output> [--keep]")
}

fn usage_give_back() -> String {
    format!("{BINARY_NAME} give-back [--abandon]")
}

fn usage_volume() -> String {
    format!("{BINARY_NAME} volume [<percent>]")
}

fn usage_settings() -> String {
    format!("{BINARY_NAME} settings")
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

fn parse_outputs_args(args: &[String]) -> std::result::Result<(), String> {
    if let Some(token) = args.first() {
        return Err(usage_error(usage_outputs(), unexpected(token)));
    }
    Ok(())
}

fn parse_switch_args(args: &[String]) -> std::result::Result<(String, bool), String> {
    let mut output = None;
    let mut keep = false;
    for token in args {
        match token.as_str() {
            "--keep" if keep => {
                return Err(usage_error(
                    usage_switch(),
                    "`--keep` may be given only once",
                ));
            }
            "--keep" => keep = true,
            other if other.starts_with('-') => {
                return Err(usage_error(usage_switch(), unexpected(other)));
            }
            other if output.is_some() => {
                return Err(usage_error(
                    usage_switch(),
                    format!("switch takes exactly one output, got an extra `{other}`"),
                ));
            }
            other => output = Some(other.to_string()),
        }
    }
    match output {
        Some(output) => Ok((output, keep)),
        None => Err(usage_error(usage_switch(), "switch requires an output")),
    }
}

fn parse_give_back_args(args: &[String]) -> std::result::Result<bool, String> {
    let mut abandon = false;
    for token in args {
        match token.as_str() {
            "--abandon" if abandon => {
                return Err(usage_error(
                    usage_give_back(),
                    "`--abandon` may be given only once",
                ));
            }
            "--abandon" => abandon = true,
            other => return Err(usage_error(usage_give_back(), unexpected(other))),
        }
    }
    Ok(abandon)
}

fn parse_volume_args(args: &[String]) -> std::result::Result<Option<u32>, String> {
    match args {
        [] => Ok(None),
        [percent] => percent
            .parse::<u32>()
            .ok()
            .filter(|percent| *percent <= crate::volume::MAX_PERCENT)
            .map(Some)
            .ok_or_else(|| {
                usage_error(
                    usage_volume(),
                    format!(
                        "the volume must be a whole number from 0 to {}, got `{percent}`",
                        crate::volume::MAX_PERCENT
                    ),
                )
            }),
        [_, extra, ..] => Err(usage_error(usage_volume(), unexpected(extra))),
    }
}

fn parse_settings_args(args: &[String]) -> std::result::Result<(), String> {
    if let Some(token) = args.first() {
        return Err(usage_error(usage_settings(), unexpected(token)));
    }
    Ok(())
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

    type Log = Arc<Mutex<Vec<String>>>;

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

    fn sentinel_handlers(log: &Log) -> Handlers {
        Handlers {
            outputs_plain: sentinel(log, "outputs"),
            outputs_json: json_sentinel(log, "outputs"),
            switch: sentinel(log, "switch"),
            give_back: sentinel(log, "give-back"),
            volume: sentinel(log, "volume"),
            settings: sentinel(log, "settings"),
        }
    }

    fn deterministic_check() -> Result<DoctorCheckResult> {
        Ok(DoctorCheckResult::ok("test_check", "ok"))
    }

    fn failing_check() -> Result<DoctorCheckResult> {
        Ok(DoctorCheckResult::fail("test_check", "broken"))
    }

    fn test_check() -> DoctorCheck {
        DoctorCheck::new(
            "test_check",
            "A deterministic test check.",
            deterministic_check,
        )
    }

    fn sentinel_app(log: &Log) -> HeadlessApp {
        app_with_handlers(sentinel_handlers(log), vec![test_check()])
    }

    #[test]
    fn help_and_command_help_are_byte_identical_in_both_orders() {
        for command in ["outputs", "switch", "give-back", "volume", "settings"] {
            let first = app().execute(["help".to_string(), command.to_string()]);
            let final_token = app().execute([command.to_string(), "help".to_string()]);

            assert_eq!(first.exit_code, EXIT_SUCCESS, "command: {command}");
            assert_eq!(first.stdout, final_token.stdout, "command: {command}");
            assert_eq!(first.stderr, final_token.stderr, "command: {command}");
        }
    }

    #[test]
    fn doctor_help_is_byte_identical_in_both_orders() {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let app = sentinel_app(&log);

        let first = app.execute(["help".to_string(), "doctor".to_string()]);
        let final_token = app.execute(["doctor".to_string(), "help".to_string()]);

        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert_eq!(first.stderr, final_token.stderr);
        assert!(first.stdout.contains("test_check"));
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn help_and_doctor_never_invoke_an_operational_handler() {
        let cases = [
            vec!["help"],
            vec!["help", "outputs"],
            vec!["outputs", "help"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
        ];

        for args in cases {
            let log: Log = Arc::new(Mutex::new(Vec::new()));
            let app = sentinel_app(&log);
            let execution = app.execute(args.iter().map(|arg| arg.to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "args: {args:?}");
            assert!(log.lock().unwrap().is_empty(), "args: {args:?}");
        }
    }

    #[test]
    fn unknown_flags_exit_with_the_usage_code_and_name_the_usage() {
        let cases = [
            (vec!["outputs", "--bogus"], usage_outputs()),
            (vec!["switch", "Luna 2", "--bogus"], usage_switch()),
            (vec!["give-back", "--bogus"], usage_give_back()),
            (vec!["settings", "--bogus"], usage_settings()),
        ];

        for (args, usage) in cases {
            let execution = app().execute(args.iter().map(|arg| arg.to_string()));

            assert_eq!(execution.exit_code, EXIT_USAGE, "args: {args:?}");
            assert!(execution.stdout.is_empty(), "args: {args:?}");
            assert!(execution.stderr.contains("unknown flag"), "args: {args:?}");
            assert!(execution.stderr.contains(usage.as_str()), "args: {args:?}");
        }
    }

    #[test]
    fn switch_without_an_output_exits_with_the_usage_code() {
        for args in [vec!["switch"], vec!["switch", "--keep"]] {
            let execution = app().execute(args.iter().map(|arg| arg.to_string()));

            assert_eq!(execution.exit_code, EXIT_USAGE, "args: {args:?}");
            assert!(
                execution.stderr.contains("switch requires an output"),
                "args: {args:?}"
            );
            assert!(
                execution.stderr.contains(usage_switch().as_str()),
                "args: {args:?}"
            );
        }
    }

    #[test]
    fn repeated_flags_exit_with_the_usage_code() {
        let cases = [
            (vec!["switch", "Luna 2", "--keep", "--keep"], usage_switch()),
            (
                vec!["give-back", "--abandon", "--abandon"],
                usage_give_back(),
            ),
        ];

        for (args, usage) in cases {
            let execution = app().execute(args.iter().map(|arg| arg.to_string()));

            assert_eq!(execution.exit_code, EXIT_USAGE, "args: {args:?}");
            assert!(execution.stderr.contains("only once"), "args: {args:?}");
            assert!(execution.stderr.contains(usage.as_str()), "args: {args:?}");
        }
    }

    #[test]
    fn extra_positionals_exit_with_the_usage_code() {
        let args = vec!["switch", "Luna 2", "Kitchen"];
        let execution = app().execute(args.iter().map(|arg| arg.to_string()));

        assert_eq!(execution.exit_code, EXIT_USAGE, "args: {args:?}");
        assert!(execution.stderr.contains("extra"), "args: {args:?}");
    }

    #[test]
    fn volume_takes_at_most_one_percent_from_0_to_100() {
        assert_eq!(parse_volume_args(&[]), Ok(None));
        assert_eq!(parse_volume_args(&["0".to_string()]), Ok(Some(0)));
        assert_eq!(parse_volume_args(&["100".to_string()]), Ok(Some(100)));
        for bad in [vec!["101"], vec!["-5"], vec!["loud"], vec!["50", "60"]] {
            let args: Vec<String> = bad.iter().map(|arg| arg.to_string()).collect();
            let execution =
                app().execute(std::iter::once("volume".to_string()).chain(args.iter().cloned()));
            assert_eq!(execution.exit_code, EXIT_USAGE, "args: {bad:?}");
        }
    }

    #[test]
    fn switch_forwards_the_output_and_keep_flag_to_the_handler() {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let switch_log = Arc::clone(&log);
        let handlers = Handlers {
            outputs_plain: sentinel(&log, "outputs"),
            outputs_json: json_sentinel(&log, "outputs"),
            switch: Box::new(move |context: &CommandContext| -> Result<Execution> {
                switch_log
                    .lock()
                    .unwrap()
                    .push(format!("switch {}", context.args().join(" ")));
                Ok(Execution::success(String::new()))
            }),
            give_back: sentinel(&log, "give-back"),
            volume: sentinel(&log, "volume"),
            settings: sentinel(&log, "settings"),
        };
        let app = app_with_handlers(handlers, vec![test_check()]);

        for args in [
            vec!["switch", "Luna 2"],
            vec!["switch", "Luna 2", "--keep"],
            vec!["switch", "--keep", "Luna 2"],
        ] {
            let execution = app.execute(args.iter().map(|arg| arg.to_string()));
            assert_eq!(execution.exit_code, EXIT_SUCCESS, "args: {args:?}");
        }

        assert_eq!(
            log.lock().unwrap().clone(),
            vec![
                "switch Luna 2".to_string(),
                "switch Luna 2 --keep".to_string(),
                "switch --keep Luna 2".to_string()
            ]
        );
    }

    #[test]
    fn json_is_accepted_on_outputs_and_doctor() {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let app = sentinel_app(&log);

        let before = app.execute(["--json".to_string(), "outputs".to_string()]);
        let after = app.execute(["outputs".to_string(), "--json".to_string()]);

        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stdout, "null\n");
        assert_eq!(
            log.lock().unwrap().clone(),
            vec!["outputs".to_string(), "outputs".to_string()]
        );

        let doctor_before = app.execute(["--json".to_string(), "doctor".to_string()]);
        let doctor_after = app.execute(["doctor".to_string(), "--json".to_string()]);

        assert_eq!(doctor_before.exit_code, EXIT_SUCCESS);
        assert_eq!(doctor_before.stdout, doctor_after.stdout);
        let report: DoctorReport =
            serde_json::from_str(&doctor_after.stdout).expect("doctor output must be valid JSON");
        assert_eq!(report.plugin_id, PLUGIN_ID);
        assert_eq!(report.checks.len(), 1);
        assert_eq!(report.checks[0].id, "test_check");
    }

    #[test]
    fn json_is_rejected_on_every_other_command() {
        let cases = [
            vec!["switch", "Luna 2", "--json"],
            vec!["give-back", "--json"],
            vec!["settings", "--json"],
        ];

        for args in cases {
            let log: Log = Arc::new(Mutex::new(Vec::new()));
            let app = sentinel_app(&log);
            let execution = app.execute(args.iter().map(|arg| arg.to_string()));

            assert_eq!(execution.exit_code, EXIT_USAGE, "args: {args:?}");
            assert!(execution.stdout.is_empty(), "args: {args:?}");
            assert!(
                execution.stderr.contains("does not support --json"),
                "args: {args:?}"
            );
            assert!(log.lock().unwrap().is_empty(), "args: {args:?}");
        }
    }

    #[test]
    fn doctor_exits_zero_for_a_valid_report_even_when_a_check_fails() {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let check = DoctorCheck::new("test_check", "A failing test check.", failing_check);
        let app = app_with_handlers(sentinel_handlers(&log), vec![check]);

        let plain = app.execute(["doctor".to_string()]);
        let json = app.execute(["doctor".to_string(), "--json".to_string()]);

        assert_eq!(plain.exit_code, EXIT_SUCCESS);
        assert!(plain.stdout.contains("fail"));
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
        let probes: [(&str, &[&str]); 5] = [
            ("outputs", &[]),
            ("switch", &["Luna 2"]),
            ("give-back", &[]),
            ("volume", &["20"]),
            ("settings", &[]),
        ];

        let mut expected = Vec::new();
        for action in manifest.executable_actions() {
            let args = manifest
                .catalog_runtime_args(&action.id)
                .expect("executable action must have runtime args");
            let command = args
                .first()
                .expect("runtime args must name a command")
                .clone();
            let probe = probes
                .iter()
                .find(|probe| probe.0 == command.as_str())
                .unwrap_or_else(|| panic!("manifest command `{command}` has no probe"));
            let mut argv = vec![command.clone()];
            argv.extend(probe.1.iter().map(|value| value.to_string()));

            let execution = app.execute(argv);
            assert_eq!(execution.exit_code, EXIT_SUCCESS, "command: {command}");
            expected.push(command);
        }

        expected.sort();
        expected.dedup();
        let mut recorded = log.lock().unwrap().clone();
        recorded.sort();
        recorded.dedup();
        assert_eq!(recorded, expected);
    }

    fn give_back_output(released: Released, abandon: bool) -> String {
        give_back_with(abandon, move |_| Ok(released))
            .expect("the substituted release must answer")
            .stdout
    }

    #[test]
    fn give_back_prints_one_truthful_line_for_every_released_outcome() {
        assert_eq!(
            give_back_output(Released::Output, false),
            "Sound no longer controls the output.\n"
        );
        assert_eq!(
            give_back_output(Released::Output, true),
            "Dropped Sound's output record.\n"
        );
        assert_eq!(
            give_back_output(Released::Nothing, false),
            "Nothing to give back: Sound was not holding the output.\n"
        );
        assert_eq!(
            give_back_output(Released::Nothing, true),
            "Nothing to give back: Sound was not holding the output.\n"
        );
    }
}
