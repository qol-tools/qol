use qol_headless::{Command, DoctorAggregateReport, HeadlessApp};

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
                        "qol-tray-doctor fix [--id <CHECK_ID>] [--apply-host-fixes] [--apply-manual-fixes] [--json]",
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

#[cfg(test)]
mod tests {
    use super::{app_with_provider, normalize_headless_args};
    use qol_headless::{
        DoctorAggregateReport, DoctorCheckResult, DoctorReport, DoctorStatus, PluginDoctorReport,
    };

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
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
