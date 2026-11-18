use std::process::ExitCode;

fn main() -> ExitCode {
    qol_log::init_stderr();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if qol_plugin_daemon::daemon::launched_as_daemon(&args) {
        return match qol_controllers::app::run_from_env() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                log::error!("{error:#}");
                ExitCode::from(1)
            }
        };
    }
    qol_controllers::cli::exit_code(args)
}

#[cfg(test)]
mod tests {
    qol_plugin_api::assert_plugin_toml_valid!();
}
