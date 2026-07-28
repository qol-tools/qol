use qol_headless::{Command, DoctorAggregateReport, HeadlessApp};

pub(super) fn try_run_from_env() -> Option<i32> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match classify_invocation(&args) {
        HostDispatch::NotCli => None,
        HostDispatch::RunHeadless => Some(run_aggregate("qol-tray", args)),
        HostDispatch::Reject => {
            eprintln!("Invalid qol-tray invocation. Run `qol-tray help` for supported forms.");
            Some(2)
        }
    }
}

pub(super) fn run_aggregate(binary_name: &str, args: Vec<String>) -> i32 {
    let args = normalize_headless_args(args);
    let execution = app(binary_name).execute(args);
    let exit_code = i32::from(execution.exit_code);
    let _ = execution.emit();
    exit_code
}

fn normalize_headless_args(args: Vec<String>) -> Vec<String> {
    args.into_iter()
        .map(|arg| {
            if arg == "-h" {
                "--help".to_string()
            } else {
                arg
            }
        })
        .collect()
}

fn app(binary_name: &str) -> HeadlessApp {
    app_with_provider(binary_name, super::aggregation::run)
}

fn app_with_provider<F>(binary_name: &str, provider: F) -> HeadlessApp
where
    F: Fn() -> anyhow::Result<DoctorAggregateReport> + Send + Sync + 'static,
{
    let app = HeadlessApp::new("qol-tray", binary_name)
        .about("Quality of Life Tray host.")
        .doctor_aggregate_provider(provider);
    if binary_name == "qol-tray-doctor" {
        return app
            .command(
                Command::new("check")
                    .about("Run the legacy host-only doctor checks.")
                    .usage(
                        "qol-tray-doctor check [--id <CHECK_ID> | --quick] [--json]",
                    )
                    .detail("Without a selector, runs every host check.")
                    .output("Plain text by default; --json returns the legacy host report.")
                    .exit_behavior("Exits 0 when healthy, 1 for warnings, and 2 for failures.")
                    .run_json(|_| anyhow::bail!("legacy check dispatch was not selected")),
            )
            .command(
                Command::new("fix")
                    .about("Run explicit legacy host repairs, then re-check.")
                    .usage(
                        "qol-tray-doctor fix [--id <CHECK_ID>] [--apply-host-fixes] [--json]",
                    )
                    .detail("Repairs are never run by the read-only `doctor` command.")
                    .output("Plain text by default; --json returns the legacy fix report.")
                    .exit_behavior(
                        "Exits 0 when the post-fix report is healthy, 1 for warnings, and 2 for failures.",
                    )
                    .run_json(|_| anyhow::bail!("legacy fix dispatch was not selected")),
            );
    }

    app.command(
        Command::new("exec")
            .about("Trigger a plugin action through the running host.")
            .usage("qol-tray exec <plugin-id> <action-id>")
            .output("Action result on stdout; diagnostics on stderr.")
            .exit_behavior("Exits non-zero when the action cannot be dispatched."),
    )
    .command(
        Command::new("open")
            .about("Open the app at an in-app route.")
            .usage("qol-tray open <route>")
            .output("No output when the route opens successfully.")
            .exit_behavior("Exits non-zero when the route cannot be opened."),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostDispatch {
    NotCli,
    RunHeadless,
    Reject,
}

fn classify_invocation(args: &[String]) -> HostDispatch {
    if args.is_empty() {
        return HostDispatch::NotCli;
    }
    if matches!(args, [argument] if matches!(argument.as_str(), "help" | "--help" | "-h")) {
        return HostDispatch::NotCli;
    }

    let tokens = args
        .iter()
        .filter(|arg| arg.as_str() != "--json")
        .map(|arg| match arg.as_str() {
            "--help" | "-h" => "help",
            token => token,
        })
        .collect::<Vec<_>>();

    let known_help_topic = |token: &str| matches!(token, "doctor" | "exec" | "open");
    let is_headless_help = matches!(
        tokens.as_slice(),
        ["help", topic, ..] if known_help_topic(topic)
    ) || matches!(
        tokens.as_slice(),
        [topic, "help"] if known_help_topic(topic)
    );

    if matches!(tokens.first(), Some(&"doctor")) || is_headless_help {
        return HostDispatch::RunHeadless;
    }

    if matches!(tokens.first(), Some(&"exec") | Some(&"open")) {
        let expected_len = if tokens.first() == Some(&"exec") {
            3
        } else {
            2
        };
        if args.iter().any(|arg| arg == "--json") || args.len() != expected_len {
            return HostDispatch::Reject;
        }
        return HostDispatch::NotCli;
    }

    if tokens.contains(&"doctor") || tokens.contains(&"help") {
        return HostDispatch::Reject;
    }

    if matches!(args, [flag] if matches!(flag.as_str(), "--version" | "-V" | "-h"))
        || matches!(args, [mode] if mode.starts_with("--write-mode="))
        || matches!(
            args,
            [courier, url]
                if courier == crate::commands::URL_COURIER_FLAG
                    && crate::commands::parse_qol_url(url).is_some()
        )
        || matches!(
            args,
            [url] if crate::commands::parse_qol_url(url).is_some()
        )
    {
        return HostDispatch::NotCli;
    }

    HostDispatch::Reject
}

#[cfg(test)]
mod tests {
    use super::{app_with_provider, classify_invocation, normalize_headless_args, HostDispatch};
    use qol_headless::{
        DoctorAggregateReport, DoctorCheckResult, DoctorReport, DoctorStatus, PluginDoctorReport,
    };

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn recognizes_every_supported_doctor_and_contextual_help_form() {
        for values in [
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
            vec!["--help", "doctor"],
            vec!["doctor", "--help"],
        ] {
            assert_eq!(
                classify_invocation(&args(&values)),
                HostDispatch::RunHeadless,
                "doctor route was not recognized: {values:?}",
            );
        }
    }

    #[test]
    fn recognizes_literal_and_contextual_help_without_starting_the_host() {
        for values in [
            vec!["help", "open"],
            vec!["open", "help"],
            vec!["help", "exec"],
            vec!["exec", "help"],
            vec!["-h", "doctor"],
            vec!["doctor", "-h"],
        ] {
            assert_eq!(
                classify_invocation(&args(&values)),
                HostDispatch::RunHeadless,
                "help route was not recognized: {values:?}",
            );
        }
    }

    #[test]
    fn leaves_daemon_and_existing_side_effecting_cli_routes_on_the_existing_path() {
        for values in [
            vec![],
            vec!["help"],
            vec!["--help"],
            vec!["-h"],
            vec!["--version"],
            vec!["--write-mode=dev"],
            vec!["exec", "plugin-test", "toggle"],
            vec!["exec", "plugin-test", "doctor"],
            vec!["exec", "plugin-test", "help"],
            vec!["open", "settings"],
            vec!["open", "doctor"],
            vec!["qol://shortcuts/add"],
            vec![crate::commands::URL_COURIER_FLAG, "qol://shortcuts/add"],
        ] {
            assert_eq!(
                classify_invocation(&args(&values)),
                HostDispatch::NotCli,
                "unrelated route was captured: {values:?}",
            );
        }
    }

    #[test]
    fn rejects_unknown_or_malformed_args_that_would_otherwise_reach_host_startup() {
        for values in [
            vec!["--bogus", "doctor"],
            vec!["help", "status", "doctor"],
            vec!["--write-mode=dev", "doctor"],
            vec!["status", "doctor"],
            vec!["--doctor"],
            vec!["--json"],
            vec!["unknown"],
            vec!["--write-mode=dev", "extra"],
            vec!["help", "status"],
            vec!["status", "help"],
            vec!["--help", "status"],
            vec!["status", "--help"],
            vec!["qol://shortcuts/add", "doctor"],
            vec![crate::commands::URL_COURIER_FLAG],
            vec![crate::commands::URL_COURIER_FLAG, "https://example.com"],
            vec!["--json", "exec", "plugin-test", "toggle"],
            vec!["exec", "plugin-test", "toggle", "--json"],
            vec!["--json", "open", "settings"],
            vec!["open", "settings", "--json"],
            vec!["open", "settings", "extra"],
            vec!["exec", "plugin-test", "toggle", "extra"],
        ] {
            assert_eq!(
                classify_invocation(&args(&values)),
                HostDispatch::Reject,
                "malformed host route was not rejected: {values:?}",
            );
        }
    }

    #[test]
    fn help_apps_document_the_real_surface_for_each_binary() {
        let host = app_with_provider("qol-tray", || Ok(sample_report())).execute(args(&["help"]));
        assert!(host.stdout.contains("exec"));
        assert!(host.stdout.contains("open"));
        assert!(host.stdout.contains("doctor"));
        assert!(!host.stdout.contains("\n  check"));
        assert!(!host.stdout.contains("\n  fix"));

        let doctor_app = || app_with_provider("qol-tray-doctor", || Ok(sample_report()));
        let doctor = doctor_app().execute(args(&["help"]));
        assert!(doctor.stdout.contains("check"));
        assert!(doctor.stdout.contains("fix"));
        assert!(doctor.stdout.contains("doctor"));
        for values in [
            vec!["check", "help"],
            vec!["help", "check"],
            vec!["check", "--help"],
            vec!["check", "-h"],
        ] {
            let check = doctor_app().execute(normalize_headless_args(args(&values)));
            assert_eq!(check.exit_code, 0, "{values:?}: {}", check.stderr);
            assert!(check.stdout.contains("check [--id <CHECK_ID> | --quick]"));
            assert!(check.stdout.contains("Supports --json"));
        }
        for values in [
            vec!["fix", "help"],
            vec!["help", "fix"],
            vec!["fix", "--help"],
            vec!["fix", "-h"],
        ] {
            let fix = doctor_app().execute(normalize_headless_args(args(&values)));
            assert_eq!(fix.exit_code, 0, "{values:?}: {}", fix.stderr);
            assert!(fix
                .stdout
                .contains("fix [--id <CHECK_ID>] [--apply-host-fixes]"));
            assert!(fix.stdout.contains("Supports --json"));
        }
    }

    #[test]
    fn production_cli_wiring_preserves_nested_json_and_grouped_plain_output() {
        let json = app_with_provider("qol-tray", || Ok(sample_report()))
            .execute(args(&["--json", "doctor"]));
        let decoded: DoctorAggregateReport = serde_json::from_str(&json.stdout).unwrap();

        assert_eq!(json.exit_code, 2);
        assert_eq!(decoded.status, DoctorStatus::Fail);
        assert_eq!(decoded.host.plugin_id, "qol-tray");
        assert_eq!(decoded.plugins[0].plugin_id, "plugin-a");
        assert_eq!(
            decoded.plugins[0].report.as_ref().unwrap().checks[0].id,
            "shared_check"
        );
        assert_eq!(decoded.plugins[1].plugin_id, "plugin-z");
        assert!(decoded.plugins[1].report.is_none());
        assert_eq!(decoded.plugins[1].diagnostics[0].id, "doctor");

        let plain =
            app_with_provider("qol-tray-doctor", || Ok(sample_report())).execute(args(&["doctor"]));
        assert_eq!(plain.exit_code, 2);
        assert!(plain.stdout.contains("Host qol-tray: warn"));
        assert!(plain.stdout.contains("Plugin plugin-a: ok"));
        assert!(plain.stdout.contains("Report plugin-a: ok"));
        assert!(plain
            .stdout
            .contains("[ok] shared_check - plugin is healthy"));
        assert!(plain.stdout.contains("Plugin plugin-z: fail"));
        assert!(plain.stdout.contains("Diagnostics:"));
        assert!(plain.stdout.contains("[fail] doctor - invocation failed"));
    }

    fn sample_report() -> DoctorAggregateReport {
        let host = DoctorReport::from_results(
            "qol-tray",
            vec![DoctorCheckResult::warn(
                "host_check",
                "host needs attention",
            )],
        );
        let plugin_a = PluginDoctorReport::new(
            "plugin-a",
            Vec::new(),
            Some(DoctorReport::from_results(
                "plugin-a",
                vec![DoctorCheckResult::ok("shared_check", "plugin is healthy")],
            )),
        );
        let plugin_z = PluginDoctorReport::new(
            "plugin-z",
            vec![DoctorCheckResult::fail("doctor", "invocation failed")],
            None,
        );
        DoctorAggregateReport::new(host, vec![plugin_z, plugin_a])
    }
}
