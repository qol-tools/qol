use std::process::ExitCode;

use anyhow::{anyhow, Result};

use crate::core::{self, CaskStatus, Disposal, Guards, RemovalOutcome, RemovalPlan};

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

fn guard_refusal(running: bool, cask: &CaskStatus, flags: &Flags) -> Option<String> {
    if running && !flags.quit && !flags.trash_anyway {
        return Some("app is running; pass --quit or --trash-anyway".into());
    }
    if matches!(cask, CaskStatus::Managed(_)) && !flags.brew && !flags.trash_anyway {
        return Some("Homebrew-managed; pass --brew or --trash-anyway".into());
    }
    None
}

fn cask_json(cask: &CaskStatus) -> serde_json::Value {
    match cask {
        CaskStatus::Managed(t) => serde_json::json!({ "state": "managed", "token": t.as_str() }),
        CaskStatus::NotManaged => serde_json::json!({ "state": "not_managed" }),
        CaskStatus::Unavailable(reason) => {
            serde_json::json!({ "state": "unavailable", "reason": reason })
        }
    }
}

fn output_json(
    plan: &RemovalPlan,
    guards: &Guards,
    outcome: Option<&RemovalOutcome>,
    dry_run: bool,
    brew: Option<&str>,
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
        "cask": cask_json(&guards.cask),
        "removed": removed,
        "failed": failed,
        "freed_bytes": outcome.map(|o| o.freed_bytes).unwrap_or(0),
        "brew": brew,
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
    if let Some(reason) = guard_refusal(guards.running, &guards.cask, flags) {
        eprintln!("removeapp: {reason}");
        return Ok(ExitCode::from(2));
    }

    let requested = if flags.trash_anyway {
        Disposal::Trash
    } else {
        disposal_from_flags(flags.force)
    };
    if !flags.yes && !confirm(&plan, requested == Disposal::Delete)? {
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
    let mut brew_token = None;
    if let CaskStatus::Managed(token) = &guards.cask {
        if flags.brew && !flags.trash_anyway {
            core::brew_uninstall(token)?;
            brew_token = Some(token.as_str().to_string());
        }
    }

    let outcome = core::remove_after_brew(&plan, requested, &guards.cask, brew_token.is_some())?;
    println!(
        "{}",
        output_json(&plan, &guards, Some(&outcome), false, brew_token.as_deref())
    );
    Ok(if outcome.failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn confirm(plan: &RemovalPlan, force: bool) -> Result<bool> {
    use std::io::Write;
    let verb = if force {
        "PERMANENTLY DELETE"
    } else {
        "move to Trash"
    };
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
        assert!(f.quit && f.brew && f.trash_anyway);
        assert_eq!(f.query.as_deref(), Some("Foo"));
    }

    #[test]
    fn guard_refusal_running_names_required_flag() {
        let flags = parse_flags(&["Foo".into(), "--yes".into()]).unwrap();
        let text = guard_refusal(true, &CaskStatus::NotManaged, &flags).expect("should refuse");
        assert!(
            text.contains("--quit") || text.contains("--trash-anyway"),
            "names a flag: {text}"
        );
    }

    #[test]
    fn guard_refusal_clears_when_flag_present() {
        let flags = parse_flags(&["Foo".into(), "--trash-anyway".into()]).unwrap();
        assert!(guard_refusal(true, &CaskStatus::NotManaged, &flags).is_none());
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
