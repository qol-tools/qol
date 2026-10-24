#![allow(clippy::print_stdout, clippy::print_stderr)]

mod help;
mod parse;
mod render;

use super::{catalog, open, remote, store};
use anyhow::{bail, Result};
use parse::{Command, Parsed};
use qol_host_defaults::{host, HostDefaults, Role};
use render::ListEntry;
use serde_json::json;

pub fn run_cli(args: &[String]) -> i32 {
    let parsed = match parse::parse(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("defaults: {message}");
            eprintln!("run `qol-tray defaults help` for the supported forms");
            return 2;
        }
    };
    if parsed.json && matches!(parsed.command, Command::Help(_)) {
        eprintln!("defaults: help has no --json form");
        return 2;
    }
    match execute(&parsed) {
        Ok(output) => {
            println!("{output}");
            0
        }
        Err(error) => {
            eprintln!("defaults: {error:#}");
            1
        }
    }
}

fn execute(parsed: &Parsed) -> Result<String> {
    let json = parsed.json;
    match &parsed.command {
        Command::Help(topic) => Ok(help::text(*topic).to_string()),
        Command::List => list(json),
        Command::Probe { role } => probe(*role, json),
        Command::Claim { role, plugin } => claim_role(*role, plugin, json),
        Command::Release { role } => release_role(*role, json),
        Command::Open { role, args } => open_role(*role, args, json),
    }
}

fn list(json: bool) -> Result<String> {
    let plugins = catalog::installed();
    let host = host();
    let ledger = store::ledger()?;
    let mut entries = Vec::new();
    for role in Role::ALL {
        let declaring = catalog::declaring(role, &plugins);
        if declaring.is_empty() {
            continue;
        }
        entries.push(ListEntry {
            role,
            plugins: declaring,
            holder: ledger.holder(role)?,
            report: host.probe(role),
        });
    }
    if json {
        return Ok(render::list_json(&entries).to_string());
    }
    Ok(render::list_text(&entries))
}

fn probe(role: Role, json: bool) -> Result<String> {
    let report = host().probe(role);
    if json {
        return Ok(render::report_json(&report).to_string());
    }
    Ok(render::report_text(&report))
}

fn claim_role(role: Role, plugin_id: &str, json: bool) -> Result<String> {
    remote::claim(plugin_id, role)?;
    if json {
        return Ok(json!({"role": role.id(), "plugin": plugin_id, "claimed": true}).to_string());
    }
    Ok(format!("{plugin_id} now handles {}", role.id()))
}

fn release_role(role: Role, json: bool) -> Result<String> {
    let label = remote::release(role)?;
    if json {
        return Ok(json!({"role": role.id(), "outcome": label}).to_string());
    }
    Ok(format!("{}: {}", role.id(), label.replace('_', " ")))
}

fn open_role(role: Role, args: &[String], json: bool) -> Result<String> {
    open::require_target(args)?;
    if role.is_slot() {
        bail!("{} is a key slot; there is nothing to open", role.id());
    }
    let ledger = store::ledger()?;
    let opened = open::open(&ledger, role, args)?;
    if json {
        return Ok(
            json!({"role": role.id(), "plugin": opened.plugin_id, "args": opened.args}).to_string(),
        );
    }
    Ok(format!(
        "{} opened {}",
        opened.plugin_id,
        opened.args.join(" ")
    ))
}
