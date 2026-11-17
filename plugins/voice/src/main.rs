use std::process::ExitCode;

fn main() -> ExitCode {
    qol_log::init_stderr();
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if qol_plugin_daemon::daemon::launched_as_daemon(&args) {
        return match qol_voice::app::run_daemon() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                log::error!("{error:#}");
                ExitCode::FAILURE
            }
        };
    }
    qol_voice::cli::exit_code(args)
}

#[cfg(test)]
mod tests {
    qol_plugin_api::assert_plugin_toml_valid!();
}
