use std::process::ExitCode;

use anyhow::{Context, Result};
use qol_headless::{
    Command, CommandContext, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput,
};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "plugin-template";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with_handlers(
        |_| Ok(PlainTextOutput::text("Hello from My Plugin")),
        |_| {
            qol_apps::desktop_integration::open_plugin_settings(PLUGIN_ID)
                .context("failed to open settings URL")?;
            Ok(PlainTextOutput::empty())
        },
    )
}

fn app_with_handlers<Run, Settings>(run: Run, settings: Settings) -> HeadlessApp
where
    Run: Fn(&CommandContext) -> Result<PlainTextOutput> + Send + Sync + 'static,
    Settings: Fn(&CommandContext) -> Result<PlainTextOutput> + Send + Sync + 'static,
{
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Run the canonical qol-tray plugin scaffold.")
        .default_command(["run"])
        .command(
            Command::new("run")
                .about("Run the plugin example action.")
                .usage(format!("{BINARY_NAME} run"))
                .output("Prints the example greeting.")
                .exit_behavior("Exits non-zero if the action cannot run.")
                .run_plain_text(run),
        )
        .command(
            Command::new("settings")
                .about("Open the plugin settings.")
                .usage(format!("{BINARY_NAME} settings"))
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero if the settings URL cannot be opened.")
                .run_plain_text(settings),
        )
        .doctor_checks(doctor_checks())
}

fn doctor_checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "platform_supported",
            "Verify the current platform is declared by the plugin.",
            platform_supported_check,
        ),
        DoctorCheck::new(
            "config_readable",
            "Verify the plugin can run without persistent config.",
            || {
                Ok(DoctorCheckResult::ok(
                    "config_readable",
                    "No persistent config is required.",
                ))
            },
        ),
    ]
}

fn platform_supported_check() -> Result<DoctorCheckResult> {
    Ok(platform_supported_result(crate::platform::current_support()))
}

fn platform_supported_result(support: crate::platform::PlatformSupport) -> DoctorCheckResult {
    if support.supported {
        return DoctorCheckResult::ok(
            "platform_supported",
            format!("{} is supported.", support.name),
        );
    }
    DoctorCheckResult::fail(
        "platform_supported",
        format!("{} is not declared by this plugin.", support.name),
    )
    .with_fix("Run the plugin on Linux or macOS.")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use qol_headless::{DoctorReport, DoctorStatus, EXIT_SUCCESS, EXIT_USAGE};

    use super::*;

    #[derive(Default)]
    struct OperationCalls {
        run: AtomicUsize,
        settings: AtomicUsize,
    }

    fn sentinel_app(calls: Arc<OperationCalls>) -> HeadlessApp {
        let run_calls = Arc::clone(&calls);
        let settings_calls = Arc::clone(&calls);
        app_with_handlers(
            move |_| {
                run_calls.run.fetch_add(1, Ordering::SeqCst);
                Ok(PlainTextOutput::text("sentinel run"))
            },
            move |_| {
                settings_calls.settings.fetch_add(1, Ordering::SeqCst);
                Ok(PlainTextOutput::empty())
            },
        )
    }

    #[test]
    fn default_and_explicit_run_preserve_the_example_output() {
        let default = app().execute(Vec::new());
        let explicit = app().execute(["run".to_string()]);

        assert_eq!(default.exit_code, EXIT_SUCCESS);
        assert_eq!(default.stdout, "Hello from My Plugin\n");
        assert_eq!(explicit, default);
    }

    #[test]
    fn settings_command_preserves_the_operational_route() {
        let calls = Arc::new(OperationCalls::default());
        let execution = sentinel_app(Arc::clone(&calls)).execute(["settings".to_string()]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(calls.run.load(Ordering::SeqCst), 0);
        assert_eq!(calls.settings.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn help_first_and_final_are_equivalent() {
        let first = app().execute(["help".to_string(), "settings".to_string()]);
        let final_token = app().execute(["settings".to_string(), "help".to_string()]);

        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("Does not support --json."));
    }

    #[test]
    fn doctor_help_first_and_final_are_equivalent() {
        let first = app().execute(["help".to_string(), "doctor".to_string()]);
        let final_token = app().execute(["doctor".to_string(), "help".to_string()]);

        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("Run read-only health checks."));
        assert!(first.stdout.contains("platform_supported"));
        assert!(first.stdout.contains("config_readable"));
    }

    #[test]
    fn doctor_json_is_stable_in_both_global_flag_positions() {
        let before = app().execute(["--json".to_string(), "doctor".to_string()]);
        let after = app().execute(["doctor".to_string(), "--json".to_string()]);

        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
        let report: DoctorReport =
            serde_json::from_str(&before.stdout).expect("doctor output must be valid JSON");
        assert_eq!(report.plugin_id, PLUGIN_ID);
        assert_eq!(report.checks.len(), 2);
        assert!(report
            .checks
            .iter()
            .all(|check| !check.id.is_empty() && !check.message.is_empty()));
        assert_eq!(
            report
                .checks
                .iter()
                .find(|check| check.id == "config_readable")
                .expect("config check must exist")
                .status,
            DoctorStatus::Ok
        );
    }

    #[test]
    fn platform_support_results_match_the_manifest_contract() {
        let cases = [
            ("linux", true, DoctorStatus::Ok, None),
            ("macos", true, DoctorStatus::Ok, None),
            (
                "windows",
                false,
                DoctorStatus::Fail,
                Some("Run the plugin on Linux or macOS."),
            ),
            (
                "other",
                false,
                DoctorStatus::Fail,
                Some("Run the plugin on Linux or macOS."),
            ),
        ];

        for (name, supported, status, fix) in cases {
            let result =
                platform_supported_result(crate::platform::PlatformSupport { name, supported });

            assert_eq!(result.status, status, "platform: {name}");
            assert_eq!(
                result.message,
                if supported {
                    format!("{name} is supported.")
                } else {
                    format!("{name} is not declared by this plugin.")
                },
                "platform: {name}"
            );
            assert_eq!(result.fix.as_deref(), fix, "platform: {name}");
        }
    }

    #[test]
    fn doctor_and_help_never_invoke_operational_handlers() {
        let cases = [
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
            vec!["help", "run"],
            vec!["settings", "help"],
        ];

        for args in cases {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "args: {args:?}");
            assert_eq!(calls.run.load(Ordering::SeqCst), 0, "args: {args:?}");
            assert_eq!(calls.settings.load(Ordering::SeqCst), 0, "args: {args:?}");
        }
    }

    #[test]
    fn unsupported_json_is_rejected_before_settings_runs() {
        let calls = Arc::new(OperationCalls::default());
        let execution = sentinel_app(Arc::clone(&calls))
            .execute(["settings".to_string(), "--json".to_string()]);

        assert_eq!(execution.exit_code, EXIT_USAGE);
        assert!(execution.stderr.contains("does not support --json"));
        assert_eq!(calls.settings.load(Ordering::SeqCst), 0);
    }
}
