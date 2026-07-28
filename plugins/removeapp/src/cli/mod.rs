use std::process::ExitCode;

use anyhow::{anyhow, Result};
use qol_headless::{Command, CommandContext, CommandResult, HeadlessApp};

use crate::core::{
    self, Disposal, Guards, ManagedPackage, PackageManager, PackageStatus, RemovalOutcome,
    RemovalPlan,
};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "removeapp";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with_handlers(open_command, scan_command, remove_command)
}

fn app_with_handlers<Open, Scan, Remove>(open: Open, scan: Scan, remove: Remove) -> HeadlessApp
where
    Open: Fn(&CommandContext) -> Result<CommandResult> + Send + Sync + 'static,
    Scan: Fn(&CommandContext) -> Result<CommandResult> + Send + Sync + 'static,
    Remove: Fn(&CommandContext) -> Result<CommandResult> + Send + Sync + 'static,
{
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Inspect installed applications and remove an app with its owned leftovers.")
        .default_command(["open"])
        .command(
            Command::new("open")
                .about("Open the Remove App picker.")
                .usage(format!("{BINARY_NAME} open"))
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero if the picker cannot be opened.")
                .run_result(open),
        )
        .command(
            Command::new("scan")
                .about("Print the read-only removal plan for one installed app.")
                .usage(format!("{BINARY_NAME} scan <app>"))
                .detail("The app query must resolve to one installed application.")
                .detail("Prints the plan without moving, deleting, or quitting anything.")
                .output("Pretty-printed removal-plan JSON on stdout.")
                .exit_behavior("Exits non-zero if inventory or plan inspection fails.")
                .run_result(scan),
        )
        .command(
            Command::new("remove")
                .about("Remove one installed app and its owned leftovers.")
                .usage(format!("{BINARY_NAME} remove <app> [flags]"))
                .detail("--dry-run prints the guarded plan without removing anything.")
                .detail("--yes skips confirmation; --force permanently deletes.")
                .detail("--quit asks a running app to exit before removal.")
                .detail("--package uninstalls a managed package; --brew is its Homebrew alias.")
                .detail("--trash-anyway bypasses running/package guards and uses Trash.")
                .output("Removal outcome JSON on stdout; prompts and diagnostics on stderr.")
                .exit_behavior("Exits non-zero on refusal, cancellation, or a failed removal.")
                .run_result(remove),
        )
        .doctor_checks(crate::doctor::checks())
}

fn open_command(context: &CommandContext) -> Result<CommandResult> {
    if !context.args().is_empty() {
        return Ok(CommandResult::usage(format!(
            "removeapp: unexpected argument {:?}",
            context.args()[0]
        )));
    }

    use qol_plugin_daemon::daemon as core_daemon;
    if core_daemon::send_action(&crate::daemon::actions::CONFIG, "open", false) {
        return Ok(CommandResult::success(""));
    }
    crate::daemon::run()?;
    Ok(CommandResult::success(""))
}

fn scan_command(context: &CommandContext) -> Result<CommandResult> {
    Ok(scan_execution(context.args()))
}

fn remove_command(context: &CommandContext) -> Result<CommandResult> {
    Ok(remove_execution(context.args()))
}

pub fn disposal_from_flags(force: bool) -> Disposal {
    if force {
        Disposal::Delete
    } else {
        Disposal::Trash
    }
}

fn plan_json(plan: &RemovalPlan) -> String {
    serde_json::to_string_pretty(plan).unwrap_or_else(|_| "{}".to_string())
}

#[derive(Debug)]
struct Flags {
    dry_run: bool,
    yes: bool,
    force: bool,
    quit: bool,
    package: bool,
    brew: bool,
    trash_anyway: bool,
    query: Option<String>,
}

fn parse_flags(args: &[String]) -> Result<Flags> {
    let mut flags = Flags {
        dry_run: false,
        yes: false,
        force: false,
        quit: false,
        package: false,
        brew: false,
        trash_anyway: false,
        query: None,
    };
    for arg in args {
        match arg.as_str() {
            "--dry-run" => flags.dry_run = true,
            "--yes" | "-y" => flags.yes = true,
            "--force" => flags.force = true,
            "--quit" => flags.quit = true,
            "--package" => flags.package = true,
            "--brew" => flags.brew = true,
            "--trash-anyway" => flags.trash_anyway = true,
            other if !other.starts_with('-') && flags.query.is_none() => {
                flags.query = Some(other.to_string());
            }
            other if other.starts_with('-') => {
                anyhow::bail!("removeapp: unknown flag {other:?}");
            }
            other => {
                anyhow::bail!("removeapp: unexpected argument {other:?}");
            }
        }
    }
    Ok(flags)
}

fn guard_refusal(running: bool, package: &PackageStatus, flags: &Flags) -> Option<String> {
    if running && !flags.quit && !flags.trash_anyway {
        return Some("app is running; pass --quit or --trash-anyway".into());
    }
    if let PackageStatus::Managed(managed) = package {
        if !package_requested(flags, managed) && !flags.trash_anyway {
            return Some(format!(
                "{}-managed; pass --package or --trash-anyway",
                managed.manager().label()
            ));
        }
    }
    None
}

fn package_requested(flags: &Flags, package: &ManagedPackage) -> bool {
    flags.package || (flags.brew && package.manager() == PackageManager::Homebrew)
}

fn package_json(package: &PackageStatus) -> serde_json::Value {
    match package {
        PackageStatus::Managed(package) => serde_json::json!({
            "state": "managed",
            "manager": package.manager(),
            "id": package.id(),
            "scope": package.scope(),
        }),
        PackageStatus::NotManaged => serde_json::json!({ "state": "not_managed" }),
        PackageStatus::Unavailable(reason) => {
            serde_json::json!({ "state": "unavailable", "reason": reason })
        }
    }
}

fn legacy_cask_json(package: &PackageStatus) -> serde_json::Value {
    match package {
        PackageStatus::Managed(package) if package.manager() == PackageManager::Homebrew => {
            serde_json::json!({ "state": "managed", "token": package.id() })
        }
        PackageStatus::NotManaged => serde_json::json!({ "state": "not_managed" }),
        PackageStatus::Unavailable(reason) => {
            serde_json::json!({ "state": "unavailable", "reason": reason })
        }
        PackageStatus::Managed(_) => serde_json::Value::Null,
    }
}

fn output_json(
    plan: &RemovalPlan,
    guards: &Guards,
    outcome: Option<&RemovalOutcome>,
    dry_run: bool,
    uninstalled_package: Option<&ManagedPackage>,
) -> String {
    let removed: Vec<String> = outcome
        .map(|o| o.removed.iter().map(|p| p.display().to_string()).collect())
        .unwrap_or_default();
    let failed: Vec<serde_json::Value> = outcome
        .map(|o| {
            o.failed
                .iter()
                .map(|(p, e)| serde_json::json!({ "path": p.display().to_string(), "error": e }))
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({
        "app": plan.app.name,
        "plan": plan,
        "running": guards.running,
        "package": package_json(&guards.package),
        "cask": legacy_cask_json(&guards.package),
        "removed": removed,
        "failed": failed,
        "freed_bytes": outcome.map(|o| o.freed_bytes).unwrap_or(0),
        "package_uninstall": uninstalled_package,
        "brew": uninstalled_package
            .filter(|package| package.manager() == PackageManager::Homebrew)
            .map(ManagedPackage::id),
        "dry_run": dry_run,
    })
    .to_string()
}

fn require_query(flags: &Flags) -> Result<&str> {
    flags
        .query
        .as_deref()
        .ok_or_else(|| anyhow!("removeapp: missing <app> argument"))
}

fn scan_execution(args: &[String]) -> CommandResult {
    let flags = match parse_flags(args) {
        Ok(flags) => flags,
        Err(error) => return CommandResult::usage(format!("{error:#}")),
    };
    match run_scan(&flags) {
        Ok(output) => CommandResult::success(format!("{output}\n")),
        Err(error) => CommandResult::runtime_error(format!("{error:#}")),
    }
}

fn run_scan(flags: &Flags) -> Result<String> {
    let inventory = core::installed_apps()?;
    let app = core::resolve_unique(&inventory, require_query(flags)?)?;
    Ok(plan_json(&core::plan(&app, &inventory)?))
}

fn remove_execution(args: &[String]) -> CommandResult {
    let flags = match parse_flags(args) {
        Ok(flags) => flags,
        Err(error) => return CommandResult::usage(format!("{error:#}")),
    };
    match run_remove(&flags) {
        Ok(execution) => execution,
        Err(error) => CommandResult::runtime_error(format!("{error:#}")),
    }
}

fn run_remove(flags: &Flags) -> Result<CommandResult> {
    let inventory = core::installed_apps()?;
    let app = core::resolve_unique(&inventory, require_query(flags)?)?;
    let plan = core::plan(&app, &inventory)?;
    let guards = core::guards(&app, &inventory);

    if flags.dry_run {
        return Ok(CommandResult::success(format!(
            "{}\n",
            output_json(&plan, &guards, None, true, None)
        )));
    }
    if let Some(reason) = guard_refusal(guards.running, &guards.package, flags) {
        return Ok(CommandResult::new("", format!("removeapp: {reason}\n"), 2));
    }

    let requested = if flags.trash_anyway {
        Disposal::Trash
    } else {
        disposal_from_flags(flags.force)
    };
    let package_action = match &guards.package {
        PackageStatus::Managed(package)
            if package_requested(flags, package) && !flags.trash_anyway =>
        {
            Some(package.clone())
        }
        _ => None,
    };
    if !flags.yes
        && !confirm(
            &plan,
            requested == Disposal::Delete,
            package_action.as_ref(),
        )?
    {
        return Ok(CommandResult::runtime_error("removeapp: aborted"));
    }

    if guards.running && flags.quit {
        core::quit_and_wait(&app)?;
    }
    if !flags.trash_anyway && core::is_running(&app) {
        anyhow::bail!(
            "removeapp: {} is still running; pass --trash-anyway to move to Trash anyway",
            app.name
        );
    }
    let mut uninstalled_package = None;
    if let Some(package) = package_action {
        core::uninstall_package(&plan, &package)?;
        uninstalled_package = Some(package);
    }

    let outcome = core::remove_after_package(
        &plan,
        requested,
        &guards.package,
        uninstalled_package.is_some(),
    )?;
    let stdout = format!(
        "{}\n",
        output_json(
            &plan,
            &guards,
            Some(&outcome),
            false,
            uninstalled_package.as_ref(),
        )
    );
    Ok(if outcome.failed.is_empty() {
        CommandResult::success(stdout)
    } else {
        CommandResult::new(stdout, "", 1)
    })
}

fn confirm(plan: &RemovalPlan, force: bool, package: Option<&ManagedPackage>) -> Result<bool> {
    use std::io::Write;
    let verb = confirmation_verb(force, package);
    eprint!(
        "{verb} {} item(s) for {}? [y/N] ",
        plan.items.len(),
        plan.app.name
    );
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

fn confirmation_verb(force: bool, package: Option<&ManagedPackage>) -> String {
    if let Some(package) = package {
        format!("UNINSTALL with {}", package.manager().label())
    } else if force {
        "PERMANENTLY DELETE".to_string()
    } else {
        "move to Trash".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use qol_headless::{DoctorReport, EXIT_SUCCESS, EXIT_USAGE};

    use super::*;
    use crate::core::{InstalledApp, Leftover, LeftoverKind, MatchKind};

    #[derive(Default)]
    struct OperationCalls {
        open: AtomicUsize,
        scan: AtomicUsize,
        remove: AtomicUsize,
    }

    fn sentinel_app(calls: Arc<OperationCalls>) -> HeadlessApp {
        let open_calls = Arc::clone(&calls);
        let scan_calls = Arc::clone(&calls);
        let remove_calls = Arc::clone(&calls);

        app_with_handlers(
            move |_| {
                open_calls.open.fetch_add(1, Ordering::SeqCst);
                Ok(CommandResult::success(""))
            },
            move |_| {
                scan_calls.scan.fetch_add(1, Ordering::SeqCst);
                Ok(CommandResult::success("{\"scan\":\"sentinel\"}\n"))
            },
            move |_| {
                remove_calls.remove.fetch_add(1, Ordering::SeqCst);
                Ok(CommandResult::success("{\"remove\":\"sentinel\"}\n"))
            },
        )
    }

    fn sample_plan() -> RemovalPlan {
        let app = InstalledApp {
            name: "Foo".into(),
            bundle_id: Some("com.acme.foo".into()),
            path: PathBuf::from("/Applications/Foo.app"),
        };
        RemovalPlan {
            items: vec![Leftover {
                path: app.path.clone(),
                kind: LeftoverKind::AppBundle,
                size_bytes: 1234,
                match_kind: MatchKind::Exact,
            }],
            app,
            total_bytes: 1234,
            snapshots: vec![],
        }
    }

    #[test]
    fn disposal_default_is_trash_force_is_delete() {
        assert_eq!(disposal_from_flags(false), Disposal::Trash);
        assert_eq!(disposal_from_flags(true), Disposal::Delete);
    }

    #[test]
    fn plan_serializes_to_json_with_total() {
        let v: serde_json::Value = serde_json::from_str(&plan_json(&sample_plan())).unwrap();
        assert_eq!(v["total_bytes"].as_u64(), Some(1234));
        assert!(v["items"].is_array());
        assert_eq!(v["app"]["name"], "Foo");
    }

    #[test]
    fn parse_flags_extracts_query_and_switches() {
        let f = parse_flags(&[
            "Foo".to_string(),
            "--force".to_string(),
            "--yes".to_string(),
        ])
        .unwrap();
        assert_eq!(f.query.as_deref(), Some("Foo"));
        assert!(f.force && f.yes && !f.dry_run);
    }

    #[test]
    fn parse_flags_reads_guard_switches() {
        let f = parse_flags(&[
            "Foo".into(),
            "--quit".into(),
            "--brew".into(),
            "--trash-anyway".into(),
        ])
        .unwrap();
        assert!(f.quit && f.brew && !f.package && f.trash_anyway);
        assert_eq!(f.query.as_deref(), Some("Foo"));
    }

    #[test]
    fn guard_refusal_running_names_required_flag() {
        let flags = parse_flags(&["Foo".into(), "--yes".into()]).unwrap();
        let text = guard_refusal(true, &PackageStatus::NotManaged, &flags).expect("should refuse");
        assert!(
            text.contains("--quit") || text.contains("--trash-anyway"),
            "names a flag: {text}"
        );
    }

    #[test]
    fn guard_refusal_clears_when_flag_present() {
        let flags = parse_flags(&["Foo".into(), "--trash-anyway".into()]).unwrap();
        assert!(guard_refusal(true, &PackageStatus::NotManaged, &flags).is_none());
    }

    #[test]
    fn package_flag_is_generic_but_brew_alias_is_homebrew_only() {
        let apt = PackageStatus::Managed(
            ManagedPackage::parse(
                PackageManager::Apt,
                "firefox",
                crate::core::PackageScope::System,
            )
            .unwrap(),
        );
        let plain = parse_flags(&["Firefox".into(), "--yes".into()]).unwrap();
        assert!(guard_refusal(false, &apt, &plain)
            .unwrap()
            .contains("--package"));

        let package = parse_flags(&["Firefox".into(), "--package".into()]).unwrap();
        assert!(guard_refusal(false, &apt, &package).is_none());

        let brew = parse_flags(&["Firefox".into(), "--brew".into()]).unwrap();
        assert!(guard_refusal(false, &apt, &brew).is_some());
        let homebrew = PackageStatus::Managed(
            ManagedPackage::parse(
                PackageManager::Homebrew,
                "firefox",
                crate::core::PackageScope::System,
            )
            .unwrap(),
        );
        assert!(guard_refusal(false, &homebrew, &brew).is_none());
    }

    #[test]
    fn package_confirmation_names_the_real_operation() {
        let package = ManagedPackage::parse(
            PackageManager::Apt,
            "firefox",
            crate::core::PackageScope::System,
        )
        .unwrap();
        assert_eq!(
            confirmation_verb(false, Some(&package)),
            "UNINSTALL with APT"
        );
        assert_eq!(confirmation_verb(false, None), "move to Trash");
        assert_eq!(confirmation_verb(true, None), "PERMANENTLY DELETE");
    }

    #[test]
    fn parse_flags_rejects_unknown_flags_before_planning() {
        let err = parse_flags(&["Foo".into(), "--dryrun".into(), "--yes".into()]).unwrap_err();
        assert!(err.to_string().contains("unknown flag"));
    }

    #[test]
    fn parse_flags_rejects_extra_positionals() {
        let err = parse_flags(&["Google".into(), "Chrome".into(), "--yes".into()]).unwrap_err();
        assert!(err.to_string().contains("unexpected argument"));
    }

    #[test]
    fn remove_returns_standard_usage_code_for_parse_errors() {
        assert_eq!(
            remove_execution(&["Foo".into(), "--dryrun".into(), "--yes".into()]).exit_code,
            EXIT_USAGE
        );
    }

    #[test]
    fn operational_routes_remain_explicit_and_default_open_is_preserved() {
        let calls = Arc::new(OperationCalls::default());
        let app = sentinel_app(Arc::clone(&calls));

        assert_eq!(app.execute(Vec::new()).exit_code, EXIT_SUCCESS);
        assert_eq!(app.execute(["open".to_string()]).exit_code, EXIT_SUCCESS);
        assert_eq!(
            app.execute(["scan".to_string(), "Foo".to_string()])
                .exit_code,
            EXIT_SUCCESS
        );
        assert_eq!(
            app.execute(["remove".to_string(), "Foo".to_string()])
                .exit_code,
            EXIT_SUCCESS
        );

        assert_eq!(calls.open.load(Ordering::SeqCst), 2);
        assert_eq!(calls.scan.load(Ordering::SeqCst), 1);
        assert_eq!(calls.remove.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn help_first_and_final_are_equivalent() {
        let first = app().execute(["help".to_string(), "remove".to_string()]);
        let final_token = app().execute(["remove".to_string(), "help".to_string()]);

        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("--trash-anyway"));
        assert!(first.stdout.contains("Does not support --json."));
    }

    #[test]
    fn doctor_help_documents_the_stable_check_ids() {
        let first = app().execute(["help".to_string(), "doctor".to_string()]);
        let final_token = app().execute(["doctor".to_string(), "help".to_string()]);

        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        for id in [
            "platform_supported",
            "config_readable",
            "inventory_readable",
            "removal_prerequisites",
        ] {
            assert!(first.stdout.contains(id), "missing check id {id}");
        }
    }

    #[test]
    fn doctor_json_matches_the_shared_contract_in_both_flag_positions() {
        let before = app().execute(["--json".to_string(), "doctor".to_string()]);
        let after = app().execute(["doctor".to_string(), "--json".to_string()]);

        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
        let report: DoctorReport =
            serde_json::from_str(&before.stdout).expect("doctor output must be valid JSON");
        assert_eq!(report.plugin_id, PLUGIN_ID);
        assert_eq!(report.checks.len(), 4);
        assert!(report
            .checks
            .iter()
            .all(|check| !check.id.is_empty() && !check.message.is_empty()));
    }

    #[test]
    fn doctor_and_help_never_invoke_operational_handlers() {
        let cases = [
            vec!["help"],
            vec!["--help"],
            vec!["help", "open"],
            vec!["open", "help"],
            vec!["help", "scan"],
            vec!["scan", "help"],
            vec!["help", "remove"],
            vec!["remove", "help"],
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
            vec!["help", "doctor", "platform_supported"],
            vec!["doctor", "platform_supported", "help"],
        ];

        for args in cases {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "args: {args:?}");
            assert_eq!(calls.open.load(Ordering::SeqCst), 0, "args: {args:?}");
            assert_eq!(calls.scan.load(Ordering::SeqCst), 0, "args: {args:?}");
            assert_eq!(calls.remove.load(Ordering::SeqCst), 0, "args: {args:?}");
        }
    }

    #[test]
    fn unsupported_json_is_rejected_before_open_runs() {
        for args in [
            vec!["--json"],
            vec!["--json", "open"],
            vec!["open", "--json"],
            vec!["--json", "help"],
        ] {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_USAGE, "args: {args:?}");
            assert!(
                execution.stderr.contains("does not support --json"),
                "args: {args:?}"
            );
            assert_eq!(calls.open.load(Ordering::SeqCst), 0, "args: {args:?}");
            assert_eq!(calls.scan.load(Ordering::SeqCst), 0, "args: {args:?}");
            assert_eq!(calls.remove.load(Ordering::SeqCst), 0, "args: {args:?}");
        }
    }
}
