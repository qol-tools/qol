use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() && std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some() {
        return match plugin_controllers::daemon::run_from_env() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error:#}");
                ExitCode::from(1)
            }
        };
    }
    plugin_controllers::cli::exit_code(args)
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }
}
