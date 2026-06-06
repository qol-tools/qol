mod cli;
mod commands;
mod dev_server;
mod host_facade;
mod platform;
mod progress;
mod setup;
mod workspace;

use anyhow::{bail, Result};
use cli::{help_text, parse_cli, print_help};
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
    let command = match args.values.first().and_then(|arg| arg.to_str()) {
        Some(command) => command,
        None => return print_help(),
    };
    let rest = &args.values[1..];
    match command {
        "setup" => setup::cmd_setup(rest, args.verbose),
        "dev" => commands::dev::run(rest, args.verbose, args.skip_plugins),
        "build" => commands::build::run(rest, args.verbose),
        "clean" => commands::clean::run(rest, args.verbose),
        "install" => commands::install::run(args.verbose),
        "trace" => commands::trace::run(rest),
        "sync" => bail!("`qol sync` is not implemented yet - use `make sync` for now"),
        "help" | "-h" | "--help" => print_help(),
        other => bail!("unknown command `{other}`\n\n{}", help_text()),
    }
}
