mod config;
mod daemon;
mod movement;
mod platform;
mod restore;
mod state_store;
mod trace;

use std::env;
use std::process::ExitCode;

use config::load_config;
use state_store::FileMinimizedStateStore;

fn main() -> ExitCode {
    let action = match env::args().nth(1) {
        Some(action) => action,
        None => {
            return exit_for(daemon::run());
        }
    };

    if action == "daemon" || action == "run" {
        return exit_for(daemon::run());
    }

    let store = FileMinimizedStateStore::new(platform::state_file_path());
    let config = load_config();

    let timer = trace::ActionTimer::start(&action);
    let result = platform::execute_action(&action, &store, &config);
    timer.finish(&result);

    if let Err(error) = result {
        eprintln!("{error}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

fn exit_for(result: Result<(), String>) -> ExitCode {
    if let Err(error) = result {
        eprintln!("{error}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    qol_plugin_api::assert_plugin_toml_valid!();
}
