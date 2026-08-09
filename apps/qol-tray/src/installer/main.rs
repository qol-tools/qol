use std::process::ExitCode;

use anyhow::Result;
use qol_headless::{Command, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput};

const APP_ID: &str = "qol-tray-install";
const BINARY_NAME: &str = "qol-tray-install";

qol_conventions::declare_build_identity!(Installer);

trait InstallerOperations: Clone + Send + Sync + 'static {
    fn apply(&self, args: Vec<String>) -> Result<()>;
    fn inspect_platform_paths(&self) -> Result<DoctorCheckResult>;
}

#[derive(Clone, Copy)]
struct ProductionOperations;

impl InstallerOperations for ProductionOperations {
    fn apply(&self, args: Vec<String>) -> Result<()> {
        qol_tray::installer::run(args)
    }

    fn inspect_platform_paths(&self) -> Result<DoctorCheckResult> {
        let result = match qol_tray::installer::check_platform_paths() {
            Ok(()) => DoctorCheckResult::ok(
                "platform_paths",
                "install and autostart paths resolve for this platform",
            ),
            Err(error) => DoctorCheckResult::fail(
                "platform_paths",
                format!("installer paths are unavailable: {error:#}"),
            )
            .with_fix("run the installer on a supported desktop platform"),
        };
        Ok(result)
    }
}

fn main() -> ExitCode {
    register_build_identity();
    app().run(normalize_legacy_argv(std::env::args().skip(1)))
}

fn app() -> HeadlessApp {
    app_with_operations(ProductionOperations)
}

fn app_with_operations<O>(operations: O) -> HeadlessApp
where
    O: InstallerOperations,
{
    let doctor_operations = operations.clone();
    HeadlessApp::new(APP_ID, BINARY_NAME)
        .about("Install the QoL Tray binary and autostart entry.")
        .default_command(["install"])
        .command(operation_command(
            "install",
            "Install QoL Tray.",
            install_usage(),
            operations.clone(),
        ))
        .fallback_command(
            Command::new("legacy")
                .about("Compatibility adapter for the historical flag-only interface.")
                .run_plain_text(move |context| {
                    operations.apply(context.args().to_vec())?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .doctor_check(DoctorCheck::new(
            "platform_paths",
            "Resolve installer target paths without changing the filesystem.",
            move || doctor_operations.inspect_platform_paths(),
        ))
}

fn install_usage() -> &'static str {
    "qol-tray-install install [--source <PATH>] [--workspace <PATH>] [--dev]"
}

fn operation_command<O>(
    name: &'static str,
    about: &'static str,
    usage: &'static str,
    operations: O,
) -> Command
where
    O: InstallerOperations,
{
    let command = Command::new(name)
        .about(about)
        .usage(usage)
        .detail("--source <PATH> selects an existing install source.")
        .detail("--workspace <PATH> installs its locally built plugin bundles.")
        .detail("--dev selects dev runtime mode and requires a dev-enabled build.");
    command
        .output("Progress is written to stdout; diagnostics are written to stderr.")
        .exit_behavior("Exits non-zero when validation or an install operation fails.")
        .run_plain_text(move |context| {
            operations.apply(context.args().to_vec())?;
            Ok(PlainTextOutput::empty())
        })
}

fn normalize_legacy_argv(args: impl IntoIterator<Item = String>) -> Vec<String> {
    let args = args
        .into_iter()
        .map(|arg| {
            if arg == "-h" {
                "--help".to_string()
            } else {
                arg
            }
        })
        .collect::<Vec<_>>();
    let first_command = args
        .iter()
        .find(|arg| arg.as_str() != "--json")
        .map(String::as_str);
    let explicit_command = matches!(first_command, Some("install" | "doctor" | "help"));
    let legacy_tokens = args
        .iter()
        .filter(|arg| arg.as_str() != "--json")
        .collect::<Vec<_>>();
    let help_positions = legacy_tokens
        .iter()
        .enumerate()
        .filter_map(|(index, arg)| (arg.as_str() == "--help").then_some(index))
        .collect::<Vec<_>>();
    let legacy_help_at_boundary = matches!(
        help_positions.as_slice(),
        [position] if *position == 0 || *position == legacy_tokens.len() - 1
    );
    if !explicit_command && legacy_help_at_boundary {
        return args
            .into_iter()
            .filter(|arg| arg == "--help" || arg == "--json")
            .collect();
    }
    args
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use qol_headless::{DoctorReport, EXIT_SUCCESS, EXIT_USAGE};

    use super::*;

    #[derive(Default)]
    struct OperationCalls {
        mutations: Mutex<Vec<Vec<String>>>,
        doctor: AtomicUsize,
    }

    #[derive(Clone)]
    struct SentinelOperations {
        calls: Arc<OperationCalls>,
    }

    impl InstallerOperations for SentinelOperations {
        fn apply(&self, args: Vec<String>) -> Result<()> {
            self.calls.mutations.lock().unwrap().push(args);
            Ok(())
        }

        fn inspect_platform_paths(&self) -> Result<DoctorCheckResult> {
            self.calls.doctor.fetch_add(1, Ordering::SeqCst);
            Ok(DoctorCheckResult::ok(
                "platform_paths",
                "sentinel paths are readable",
            ))
        }
    }

    fn sentinel() -> (HeadlessApp, Arc<OperationCalls>) {
        let calls = Arc::new(OperationCalls::default());
        let operations = SentinelOperations {
            calls: Arc::clone(&calls),
        };
        (app_with_operations(operations), calls)
    }

    fn execute(app: &HeadlessApp, args: &[&str]) -> qol_headless::Execution {
        app.execute(normalize_legacy_argv(
            args.iter().map(|arg| (*arg).to_string()),
        ))
    }

    #[test]
    fn default_explicit_and_legacy_invocations_reach_the_same_operation() {
        let (app, calls) = sentinel();

        assert_eq!(execute(&app, &[]).exit_code, EXIT_SUCCESS);
        assert_eq!(
            execute(&app, &["install", "--source", "/tmp/qol-tray"]).exit_code,
            EXIT_SUCCESS
        );
        assert_eq!(execute(&app, &["--dev"]).exit_code, EXIT_SUCCESS);

        assert_eq!(
            calls.mutations.lock().unwrap().as_slice(),
            [
                Vec::<String>::new(),
                vec!["--source".to_string(), "/tmp/qol-tray".to_string()],
                vec!["--dev".to_string()],
            ]
        );
    }

    #[test]
    fn help_in_both_contextual_positions_never_mutates() {
        let (app, calls) = sentinel();

        for args in [
            &["help", "install"][..],
            &["install", "help"][..],
            &["--help"][..],
            &["--dev", "-h"][..],
            &["-h", "--dev"][..],
        ] {
            let execution = execute(&app, args);
            assert_eq!(execution.exit_code, EXIT_SUCCESS, "{args:?}");
            assert!(execution.stderr.is_empty(), "{args:?}");
        }

        assert!(calls.mutations.lock().unwrap().is_empty());
        assert_eq!(calls.doctor.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn help_in_the_middle_is_rejected_before_mutation() {
        let (app, calls) = sentinel();

        for args in [
            &["install", "help", "--dev"][..],
            &["--source", "/tmp/qol-tray", "--help", "--dev"][..],
        ] {
            let execution = execute(&app, args);
            assert_eq!(execution.exit_code, EXIT_USAGE, "{args:?}");
            assert!(
                execution
                    .stderr
                    .contains("must be the first token or final token"),
                "{args:?}"
            );
        }

        assert!(calls.mutations.lock().unwrap().is_empty());
        assert_eq!(calls.doctor.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn doctor_json_works_in_both_global_flag_positions_without_mutating() {
        let (app, calls) = sentinel();

        for args in [&["--json", "doctor"][..], &["doctor", "--json"][..]] {
            let execution = execute(&app, args);
            assert_eq!(execution.exit_code, EXIT_SUCCESS, "{args:?}");
            let report: DoctorReport = serde_json::from_str(&execution.stdout).unwrap();
            assert_eq!(report.plugin_id, APP_ID);
            assert_eq!(report.checks[0].id, "platform_paths");
        }

        assert!(calls.mutations.lock().unwrap().is_empty());
        assert_eq!(calls.doctor.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn unsupported_json_is_rejected_before_explicit_or_legacy_mutation() {
        let (app, calls) = sentinel();

        for args in [
            &["--json", "install"][..],
            &["install", "--json"][..],
            &["--json", "--dev"][..],
            &["--dev", "--json"][..],
        ] {
            let execution = execute(&app, args);
            assert_eq!(execution.exit_code, EXIT_USAGE, "{args:?}");
            assert!(
                execution.stderr.contains("does not support --json"),
                "{args:?}"
            );
        }

        assert!(calls.mutations.lock().unwrap().is_empty());
    }
}
