use anyhow::{anyhow, bail, Result};
use std::ffi::OsString;

mod contract;

pub(crate) struct CliArgs {
    pub(crate) values: Vec<OsString>,
    pub(crate) verbose: bool,
    pub(crate) skip_plugins: bool,
    pub(crate) json: bool,
}

pub(crate) fn parse_cli(args: Vec<OsString>) -> CliArgs {
    let mut values = Vec::new();
    let mut verbose = false;
    let mut skip_plugins = false;
    let mut json = false;
    let mut options = true;
    for arg in args {
        if options && arg == "--" {
            options = false;
            if !values.is_empty() {
                values.push(arg);
            }
            continue;
        }
        match options.then(|| arg.to_str()).flatten() {
            Some("--verbose" | "-v") => {
                verbose = true;
                continue;
            }
            Some("--no-plugins" | "-n") => {
                skip_plugins = true;
                continue;
            }
            Some("--json") => {
                json = true;
                continue;
            }
            _ => {}
        }
        values.push(arg);
    }
    CliArgs {
        values,
        verbose,
        skip_plugins,
        json,
    }
}

pub(crate) fn contract_execution(args: &CliArgs) -> Result<Option<qol_headless::Execution>> {
    contract::execution(args)
}

pub(crate) fn help_only(args: &[OsString]) -> bool {
    args.len() == 1
        && args[0]
            .to_str()
            .is_some_and(|argument| matches!(argument, "help" | "-h" | "--help"))
}

pub(crate) fn help_text() -> String {
    contract::general_help()
}

pub(crate) fn optional_single_arg<'a>(
    args: &'a [OsString],
    usage: &str,
) -> Result<Option<&'a str>> {
    if args.len() > 1 {
        bail!("usage: {usage}");
    }
    let arg = match args.first() {
        Some(arg) => arg,
        None => return Ok(None),
    };
    let value = arg
        .to_str()
        .ok_or_else(|| anyhow!("argument is not valid UTF-8"))?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_verbose_flag_before_command() {
        let args = parse_cli(vec!["--verbose".into(), "install".into()]);
        assert!(args.verbose);
        assert!(!args.json);
        assert_eq!(args.values, vec![OsString::from("install")]);
    }

    #[test]
    fn parses_verbose_flag_after_command() {
        let args = parse_cli(vec!["install".into(), "-v".into()]);
        assert!(args.verbose);
        assert_eq!(args.values, vec![OsString::from("install")]);
    }

    #[test]
    fn preserves_delimiter_and_arguments_after_command() {
        let args = parse_cli(vec![
            "emu".into(),
            "sh".into(),
            "run-a".into(),
            "--".into(),
            "echo".into(),
            "-v".into(),
            "--no-plugins".into(),
        ]);
        assert!(!args.verbose);
        assert!(!args.skip_plugins);
        assert_eq!(
            args.values,
            ["emu", "sh", "run-a", "--", "echo", "-v", "--no-plugins"].map(OsString::from)
        );
    }

    #[test]
    fn leading_delimiter_stops_global_parsing_without_becoming_the_command() {
        let args = parse_cli(vec!["--".into(), "install".into(), "-v".into()]);
        assert!(!args.verbose);
        assert!(!args.json);
        assert_eq!(args.values, ["install", "-v"].map(OsString::from));
    }

    #[test]
    fn parses_json_as_a_global_flag_on_either_side_of_doctor() {
        for values in [
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["doctor", "check-id", "--json"],
        ] {
            let args = parse_cli(values.iter().copied().map(OsString::from).collect());
            assert!(args.json, "{values:?}");
            assert!(!args.values.iter().any(|value| value == "--json"));
        }
    }

    #[test]
    fn delimiter_preserves_json_as_a_command_argument() {
        let args = parse_cli(vec!["doctor".into(), "--".into(), "--json".into()]);
        assert!(!args.json);
        assert_eq!(args.values, ["doctor", "--", "--json"].map(OsString::from));
    }

    #[test]
    fn recognizes_only_a_single_help_argument() {
        for argument in ["help", "-h", "--help"] {
            assert!(help_only(&[OsString::from(argument)]));
        }
        assert!(!help_only(&[]));
        assert!(!help_only(&["--help".into(), "extra".into()]));
        assert!(!help_only(&["other".into()]));
    }

    #[test]
    fn contextual_check_help_advertises_exact_staged_checks() {
        let args = parse_cli(["help", "check"].into_iter().map(OsString::from).collect());
        let execution = contract_execution(&args).unwrap().unwrap();
        assert!(execution.stdout.contains("qol check [--staged]"));
    }

    #[test]
    fn general_help_documents_every_global_flag() {
        let args = parse_cli(vec!["help".into()]);
        let execution = contract_execution(&args).unwrap().unwrap();

        for flag in ["-v, --verbose", "-n, --no-plugins", "--json", "--"] {
            assert!(execution.stdout.contains(flag), "{flag}");
        }
    }

    #[test]
    fn contextual_doctor_help_is_equivalent_in_first_and_final_positions() {
        let first = parse_cli(["help", "doctor"].into_iter().map(OsString::from).collect());
        let final_position =
            parse_cli(["doctor", "help"].into_iter().map(OsString::from).collect());

        let first = contract_execution(&first).unwrap().unwrap();
        let final_position = contract_execution(&final_position).unwrap().unwrap();

        assert_eq!(first.exit_code, qol_headless::EXIT_SUCCESS);
        assert_eq!(first, final_position);
        assert!(first
            .stdout
            .contains("Run read-only host and plugin health checks."));
        assert!(first.stdout.contains("qol doctor <step>"));
        assert!(first.stdout.contains("Aggregate JSON"));
        assert!(first.stdout.contains("Legacy JSON"));
    }

    #[test]
    fn help_in_the_middle_is_rejected() {
        let args = parse_cli(
            ["doctor", "help", "autostart_target"]
                .into_iter()
                .map(OsString::from)
                .collect(),
        );
        let execution = contract_execution(&args).unwrap().unwrap();

        assert_eq!(execution.exit_code, qol_headless::EXIT_USAGE);
        assert!(execution.stderr.contains("first token or final token"));
    }

    #[test]
    fn json_is_rejected_for_commands_without_a_structured_contract() {
        let args = parse_cli(
            ["build", "--json"]
                .into_iter()
                .map(OsString::from)
                .collect(),
        );
        let execution = contract_execution(&args).unwrap().unwrap();

        assert_eq!(execution.exit_code, qol_headless::EXIT_USAGE);
        assert!(execution.stderr.contains("does not support --json"));
    }

    #[test]
    fn doctor_json_is_left_for_the_real_doctor_dispatch() {
        for values in [["--json", "doctor"], ["doctor", "--json"]] {
            let args = parse_cli(values.into_iter().map(OsString::from).collect());
            assert!(contract_execution(&args).unwrap().is_none());
        }
    }

    #[test]
    fn sessions_help_lists_every_subcommand() {
        let args = parse_cli(vec!["help".into(), "sessions".into()]);
        let execution = contract_execution(&args).unwrap().unwrap();
        assert_eq!(execution.exit_code, qol_headless::EXIT_SUCCESS);
        let usage = format!(
            "qol sessions <{}>",
            crate::commands::sessions::SUBCOMMANDS
                .iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>()
                .join("|")
        );
        assert!(execution.stdout.contains(&usage));
        assert!(execution
            .stdout
            .contains("sessions_list, session_spawn, session_bridge, and session_loop_close"));
        assert!(execution
            .stdout
            .contains("read, send, wait, and focus remain human diagnostics"));
    }

    #[test]
    fn session_spawn_help_describes_the_orchestration() {
        let args = parse_cli(
            ["sessions", "spawn", "--help"]
                .into_iter()
                .map(OsString::from)
                .collect(),
        );
        let execution = contract_execution(&args).unwrap().unwrap();
        assert_eq!(execution.exit_code, qol_headless::EXIT_SUCCESS);
        assert!(execution.stdout.contains(
            "qol sessions spawn --tool TOOL --cwd PATH [--key KEY] [--surface tab|os-window]"
        ));
        assert!(execution
            .stdout
            .contains("generates a key when --key is omitted"));
        assert!(execution
            .stdout
            .contains("~/.config/qol-tray/sessions.toml"));
    }

    #[test]
    fn session_bridge_help_describes_the_atomic_transaction() {
        let args = parse_cli(
            ["sessions", "bridge", "--help"]
                .into_iter()
                .map(OsString::from)
                .collect(),
        );
        let execution = contract_execution(&args).unwrap().unwrap();
        assert_eq!(execution.exit_code, qol_headless::EXIT_SUCCESS);
        assert!(execution
            .stdout
            .contains("qol sessions bridge <session> <task...> [--timeout-ms N]"));
        assert!(execution.stdout.contains("Submits exactly once"));
        assert!(execution.stdout.contains("completion_marker"));
        assert!(execution.stdout.contains("Timeout defaults to 24h"));
    }
}
