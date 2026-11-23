#![allow(clippy::print_stdout, clippy::print_stderr)]

mod cli;
mod commands;
mod dev_console;
mod dev_server;
mod dev_shutdown;
mod host_facade;
mod installed_tray;
mod platform;
mod poller;
mod process_guardian;
mod progress;
mod self_exec;
mod setup;
mod workspace;

use std::env;

fn main() {
    qol_log::init_stderr();
    if let Err(error) = cli::run(env::args_os().skip(1).collect()) {
        eprintln!("{error:#}");
        std::process::exit(commands::pr::exit_code(&error));
    }
}
