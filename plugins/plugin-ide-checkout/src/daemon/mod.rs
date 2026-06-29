mod checkout;
mod config;
mod server;
mod takeover;

use std::process::ExitCode;

pub fn run() -> ExitCode {
    let config = config::Config::load();
    match server::serve(daemon_port(), config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[task-runner] failed to start daemon: {error}");
            ExitCode::from(1)
        }
    }
}

fn daemon_port() -> u16 {
    env!("QOL_DAEMON_PORT")
        .parse()
        .expect("QOL_DAEMON_PORT must be a valid u16")
}
