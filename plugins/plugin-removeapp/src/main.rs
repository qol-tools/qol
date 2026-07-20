use std::env;
use std::process::ExitCode;

use plugin_removeapp::{cli, daemon};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("open") => open(),
        Some("scan") => cli::scan(&args[1..]),
        Some("remove") => cli::remove(&args[1..]),
        Some(other) => {
            eprintln!("removeapp: unknown subcommand {other:?}");
            ExitCode::from(2)
        }
    }
}

fn open() -> ExitCode {
    use qol_plugin_daemon::daemon as core_daemon;
    if core_daemon::send_action(&daemon::actions::CONFIG, "open", false) {
        return ExitCode::SUCCESS;
    }
    match daemon::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("removeapp: {e:#}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    qol_plugin_api::assert_plugin_toml_valid!();
}
