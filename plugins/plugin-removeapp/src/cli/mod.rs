use std::process::ExitCode;

use anyhow::{anyhow, Result};

use crate::core::{
    self, Disposal, Guards, ManagedPackage, PackageManager, PackageStatus, RemovalOutcome,
    RemovalPlan,
};

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

pub fn scan(args: &[String]) -> ExitCode {
    let flags = match parse_flags(args) {
        Ok(flags) => flags,
        Err(e) => {
            eprintln!("{e:#}");
            return ExitCode::from(2);
        }
    };
    match run_scan(&flags) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::from(1)
        }
    }
}

fn run_scan(flags: &Flags) -> Result<()> {
    let inventory = core::installed_apps()?;
    let app = core::resolve_unique(&inventory, require_query(flags)?)?;
    println!("{}", plan_json(&core::plan(&app, &inventory)?));
    Ok(())
}

pub fn remove(args: &[String]) -> ExitCode {
    let flags = match parse_flags(args) {
        Ok(flags) => flags,
        Err(e) => {
            eprintln!("{e:#}");
            return ExitCode::from(2);
        }
    };
    match run_remove(&flags) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::from(1)
        }
    }
}

fn run_remove(flags: &Flags) -> Result<ExitCode> {
    let inventory = core::installed_apps()?;
    let app = core::resolve_unique(&inventory, require_query(flags)?)?;
    let plan = core::plan(&app, &inventory)?;
    let guards = core::guards(&app, &inventory);

    if flags.dry_run {
        println!("{}", output_json(&plan, &guards, None, true, None));
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(reason) = guard_refusal(guards.running, &guards.package, flags) {
        eprintln!("removeapp: {reason}");
        return Ok(ExitCode::from(2));
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
        eprintln!("removeapp: aborted");
        return Ok(ExitCode::from(1));
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
    println!(
        "{}",
        output_json(
            &plan,
            &guards,
            Some(&outcome),
            false,
            uninstalled_package.as_ref(),
        )
    );
    Ok(if outcome.failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
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
    use super::*;
    use crate::core::{InstalledApp, Leftover, LeftoverKind, MatchKind};
    use std::path::PathBuf;

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
    fn remove_returns_usage_code_for_parse_errors() {
        assert_eq!(
            remove(&["Foo".into(), "--dryrun".into(), "--yes".into()]),
            ExitCode::from(2)
        );
    }
}
