mod cli;
mod commands;
mod dev_console;
mod dev_server;
mod dev_shutdown;
mod host_facade;
mod platform;
mod poller;
mod process_guardian;
mod progress;
mod self_exec;
mod setup;
mod workspace;

use anyhow::{bail, Result};
use cli::{contract_execution, help_text, parse_cli};
use std::env;
use std::ffi::OsString;

fn main() {
    if let Err(error) = run(env::args_os().skip(1).collect()) {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run(args: Vec<OsString>) -> Result<()> {
    let args = parse_cli(args);
    if let Some(execution) = contract_execution(&args)? {
        let exit_code = execution.exit_code;
        let _ = execution.emit();
        if exit_code != qol_headless::EXIT_SUCCESS {
            std::process::exit(i32::from(exit_code));
        }
        return Ok(());
    }
    let command = match args.values.first().and_then(|arg| arg.to_str()) {
        Some(command) => command,
        None => return Ok(()),
    };
    let rest = &args.values[1..];
    match command {
        qol_process::PROCESS_TREE_GUARDIAN_COMMAND => process_guardian::run(),
        qol_dev_orchestrator::FLOW_WORKER_COMMAND => {
            commands::flow::run_worker(rest, &env::current_exe()?)
        }
        qol_dev_orchestrator::IMAGE_IMPORT_WORKER_COMMAND => {
            commands::env::run_image_import_worker(rest)
        }
        "setup" => setup::cmd_setup(rest, args.verbose),
        "dev" => commands::dev::run(rest, args.verbose, args.skip_plugins),
        command if command == commands::dev::DEV_PREBUILD_COMMAND => {
            commands::dev::prebuild(rest, args.verbose, args.skip_plugins)
        }
        "emu" => commands::emu::run(rest, args.verbose),
        "env" => commands::env::run(rest, args.verbose),
        "flow" => commands::flow::run(rest, args.verbose),
        "cat" => commands::cat::run(rest),
        "build" => commands::build::run(rest, args.verbose),
        "check" => commands::check::run(rest, args.verbose),
        "clean" => commands::clean::run(rest, args.verbose),
        "install" => commands::install::run(rest, args.verbose),
        "sessions" => commands::sessions::run(
            rest,
            if args.json {
                qol_headless::OutputFormat::Json
            } else {
                qol_headless::OutputFormat::PlainText
            },
        ),
        "trace" => commands::trace::run(rest),
        "trace-rs" => commands::trace_rs::run(rest),
        "doctor" => commands::doctor::run(
            rest,
            if args.json {
                qol_headless::OutputFormat::Json
            } else {
                qol_headless::OutputFormat::PlainText
            },
        ),
        "sync" => {
            if args.json {
                let value = commands::sync::run_json()?;
                println!("{value}");
            } else {
                commands::sync::run(rest)?;
            }
            Ok(())
        }
        "help" | "-h" | "--help" => Ok(()),
        other => bail!("unknown command `{other}`\n\n{}", help_text()),
    }
}
