use std::process::ExitCode;

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if matches!(args.as_slice(), [arg] if arg == plugin_bluetooth::SETTINGS_SURFACE_ARG) {
        return match plugin_bluetooth::show_settings() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error:#}");
                ExitCode::from(1)
            }
        };
    }
    if args.is_empty() && std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some() {
        return match plugin_bluetooth::platform::run_daemon(plugin_bluetooth::config::load()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error:#}");
                ExitCode::from(1)
            }
        };
    }
    plugin_bluetooth::cli::exit_code(args)
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }
}
