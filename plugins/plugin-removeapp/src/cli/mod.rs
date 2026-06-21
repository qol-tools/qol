use std::process::ExitCode;

use anyhow::{anyhow, Result};

use crate::core::{self, Disposal, RemovalPlan};

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

struct Flags {
    dry_run: bool,
    yes: bool,
    force: bool,
    query: Option<String>,
}

fn parse_flags(args: &[String]) -> Flags {
    let mut flags = Flags {
        dry_run: false,
        yes: false,
        force: false,
        query: None,
    };
    for arg in args {
        match arg.as_str() {
            "--dry-run" => flags.dry_run = true,
            "--yes" | "-y" => flags.yes = true,
            "--force" => flags.force = true,
            other if !other.starts_with('-') && flags.query.is_none() => {
                flags.query = Some(other.to_string());
            }
            _ => {}
        }
    }
    flags
}

fn require_query(flags: &Flags) -> Result<&str> {
    flags
        .query
        .as_deref()
        .ok_or_else(|| anyhow!("removeapp: missing <app> argument"))
}

pub fn scan(args: &[String]) -> ExitCode {
    match run_scan(&parse_flags(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::from(1)
        }
    }
}

fn run_scan(flags: &Flags) -> Result<()> {
    let app = core::resolve_unique(require_query(flags)?)?;
    println!("{}", plan_json(&core::plan(&app)?));
    Ok(())
}

pub fn remove(args: &[String]) -> ExitCode {
    match run_remove(&parse_flags(args)) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::from(1)
        }
    }
}

fn run_remove(flags: &Flags) -> Result<ExitCode> {
    let app = core::resolve_unique(require_query(flags)?)?;
    let plan = core::plan(&app)?;
    println!("{}", plan_json(&plan));
    if flags.dry_run {
        return Ok(ExitCode::SUCCESS);
    }
    if !flags.yes && !confirm(&plan, flags.force)? {
        eprintln!("removeapp: aborted");
        return Ok(ExitCode::from(1));
    }
    let outcome = core::remove(&plan, disposal_from_flags(flags.force))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&outcome).unwrap_or_default()
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
        ]);
        assert_eq!(f.query.as_deref(), Some("Foo"));
        assert!(f.force && f.yes && !f.dry_run);
    }
}
