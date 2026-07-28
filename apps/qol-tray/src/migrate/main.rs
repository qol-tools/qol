use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use qol_headless::{Command, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput};

const APP_ID: &str = "qol-tray-migrate";
const BINARY_NAME: &str = "qol-tray-migrate";

trait MigrationOperations: Clone + Send + Sync + 'static {
    fn apply(&self, args: Vec<String>) -> Result<()>;
    fn inspect_config_dir(&self) -> Result<DoctorCheckResult>;
}

#[derive(Clone, Copy)]
struct ProductionOperations;

impl MigrationOperations for ProductionOperations {
    fn apply(&self, args: Vec<String>) -> Result<()> {
        run_migrations(args)
    }

    fn inspect_config_dir(&self) -> Result<DoctorCheckResult> {
        let config_dir =
            qol_tray::paths::shared_config_dir().context("locating qol-tray config dir")?;
        Ok(config_dir_result(&config_dir))
    }
}

fn main() -> ExitCode {
    app().run(normalize_legacy_argv(std::env::args().skip(1)))
}

fn app() -> HeadlessApp {
    app_with_operations(ProductionOperations)
}

fn app_with_operations<O>(operations: O) -> HeadlessApp
where
    O: MigrationOperations,
{
    let explicit_operations = operations.clone();
    let legacy_operations = operations.clone();
    let doctor_operations = operations;
    HeadlessApp::new(APP_ID, BINARY_NAME)
        .about("Run QoL Tray config and cloud-data migrations.")
        .default_command(["run"])
        .command(
            Command::new("run")
                .about("Run pending migrations.")
                .usage("qol-tray-migrate run [--config-dir <PATH>] [--post-auth]")
                .detail("--config-dir <PATH> selects a specific config directory.")
                .detail("--post-auth also runs cloud migrations when signed in.")
                .output("Applied migrations are written to stdout.")
                .exit_behavior("Exits non-zero when migration setup or execution fails.")
                .run_plain_text(move |context| {
                    explicit_operations.apply(context.args().to_vec())?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .fallback_command(
            Command::new("legacy")
                .about("Compatibility adapter for the historical flag-only interface.")
                .run_plain_text(move |context| {
                    legacy_operations.apply(context.args().to_vec())?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .doctor_check(DoctorCheck::new(
            "config_dir",
            "Inspect the migration config path without running migrations.",
            move || doctor_operations.inspect_config_dir(),
        ))
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
    let explicit_command = matches!(first_command, Some("run" | "doctor" | "help"));
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

fn run_migrations(args: impl IntoIterator<Item = String>) -> Result<()> {
    let args = parse_args(args)?;
    let config_dir = match args.config_dir {
        Some(dir) => dir,
        None => qol_tray::paths::shared_config_dir().context("locating qol-tray config dir")?,
    };
    if !config_dir.exists() {
        eprintln!("config dir does not exist: {}", config_dir.display());
        return Ok(());
    }

    let pre_flight_reports =
        qol_migrations::run_pre_flight(&config_dir, env!("CARGO_PKG_VERSION"))?;
    print_reports("pre-flight", &pre_flight_reports);

    if args.post_auth {
        run_post_auth_blocking(&config_dir)?;
    }

    if pre_flight_reports.is_empty() && !args.post_auth {
        println!(
            "qol-tray-migrate: nothing to migrate in {}",
            config_dir.display()
        );
    }

    Ok(())
}

fn print_reports(phase: &str, reports: &[qol_migrations::MigrationReport]) {
    for report in reports {
        println!(
            "qol-tray-migrate[{phase}]: applied {} (archived {} paths)",
            report.name,
            report.archived.len(),
        );
        for path in &report.archived {
            println!("    - {}", path.display());
        }
    }
}

fn run_post_auth_blocking(config_dir: &Path) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime for post-auth")?;
    rt.block_on(qol_tray::migrations_startup::run_post_auth_if_authed(
        config_dir,
    ))
}

#[derive(Debug, PartialEq, Eq)]
struct Args {
    config_dir: Option<PathBuf>,
    post_auth: bool,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args> {
    let mut config_dir = None;
    let mut post_auth = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config-dir" => {
                let value = args
                    .next()
                    .context("--config-dir requires a path argument")?;
                config_dir = Some(PathBuf::from(value));
            }
            "--post-auth" => post_auth = true,
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(Args {
        config_dir,
        post_auth,
    })
}

fn config_dir_result(config_dir: &Path) -> DoctorCheckResult {
    match fs::metadata(config_dir) {
        Ok(metadata) if metadata.is_dir() => {
            DoctorCheckResult::ok("config_dir", "the migration config directory is readable")
        }
        Ok(_) => DoctorCheckResult::fail(
            "config_dir",
            "the migration config path exists but is not a directory",
        )
        .with_fix("replace the path with a directory before running migrations"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DoctorCheckResult::warn(
            "config_dir",
            "the migration config directory does not exist; there is nothing to migrate",
        ),
        Err(error) => DoctorCheckResult::fail(
            "config_dir",
            format!("the migration config directory cannot be inspected: {error}"),
        )
        .with_fix("check the directory permissions before running migrations"),
    }
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

    impl MigrationOperations for SentinelOperations {
        fn apply(&self, args: Vec<String>) -> Result<()> {
            self.calls.mutations.lock().unwrap().push(args);
            Ok(())
        }

        fn inspect_config_dir(&self) -> Result<DoctorCheckResult> {
            self.calls.doctor.fetch_add(1, Ordering::SeqCst);
            Ok(DoctorCheckResult::ok(
                "config_dir",
                "sentinel config is readable",
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
    fn parser_preserves_the_legacy_flag_only_contract() {
        let cases = [
            (
                Vec::<String>::new(),
                Args {
                    config_dir: None,
                    post_auth: false,
                },
            ),
            (
                vec!["--config-dir".to_string(), "/tmp/config".to_string()],
                Args {
                    config_dir: Some(PathBuf::from("/tmp/config")),
                    post_auth: false,
                },
            ),
            (
                vec![
                    "--post-auth".to_string(),
                    "--config-dir".to_string(),
                    "/tmp/config".to_string(),
                ],
                Args {
                    config_dir: Some(PathBuf::from("/tmp/config")),
                    post_auth: true,
                },
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(parse_args(input).unwrap(), expected);
        }
    }

    #[test]
    fn default_explicit_and_legacy_invocations_reach_the_same_operation() {
        let (app, calls) = sentinel();

        assert_eq!(execute(&app, &[]).exit_code, EXIT_SUCCESS);
        assert_eq!(
            execute(&app, &["run", "--config-dir", "/tmp/config"]).exit_code,
            EXIT_SUCCESS
        );
        assert_eq!(
            execute(&app, &["--config-dir", "/tmp/config", "--post-auth"]).exit_code,
            EXIT_SUCCESS
        );

        assert_eq!(
            calls.mutations.lock().unwrap().as_slice(),
            [
                Vec::<String>::new(),
                vec!["--config-dir".to_string(), "/tmp/config".to_string()],
                vec![
                    "--config-dir".to_string(),
                    "/tmp/config".to_string(),
                    "--post-auth".to_string()
                ],
            ]
        );
    }

    #[test]
    fn help_in_both_contextual_positions_never_mutates() {
        let (app, calls) = sentinel();

        for args in [
            &["help", "run"][..],
            &["run", "help"][..],
            &["--help"][..],
            &["--post-auth", "-h"][..],
            &["-h", "--post-auth"][..],
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
            &["run", "help", "--post-auth"][..],
            &["--post-auth", "--help", "--config-dir", "/tmp/config"][..],
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
            assert_eq!(report.checks[0].id, "config_dir");
        }

        assert!(calls.mutations.lock().unwrap().is_empty());
        assert_eq!(calls.doctor.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn unsupported_json_is_rejected_before_explicit_or_legacy_mutation() {
        let (app, calls) = sentinel();

        for args in [
            &["--json", "run"][..],
            &["run", "--json"][..],
            &["--json", "--post-auth"][..],
            &["--post-auth", "--json"][..],
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
