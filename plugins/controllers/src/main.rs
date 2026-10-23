use std::process::ExitCode;

fn main() -> ExitCode {
    qol_log::init_stderr();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() && std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some() {
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

    #[test]
    fn status_row_hides_when_no_controller_needs_a_fix() {
        let contract = include_str!("../qol-config.toml");
        let status = contract
            .split("[field.fixes_status]")
            .nth(1)
            .and_then(|rest| rest.split("\n[").next())
            .expect("fixes_status field exists");
        assert!(status.contains(r#"hide_values = ["none", "ok"]"#));
    }
}
