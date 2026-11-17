use std::process::ExitCode;

fn main() -> ExitCode {
    qol_log::init_stderr();
    if let Some(exit_code) = qol_shot::run_internal_mode() {
        return exit_code;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();

    if should_run_daemon(&args) {
        qol_runtime::probe!(
            "SHOT_ENTRY",
            "mode=daemon args={} socket_env={} replace_env={}",
            args.len(),
            qol_plugin_daemon::daemon::handed_socket().is_ok(),
            qol_plugin_daemon::daemon::asked_to_replace_existing()
        );
        qol_shot::app::run();
        return ExitCode::SUCCESS;
    }

    qol_runtime::probe!(
        "SHOT_ENTRY",
        "mode=cli args={} socket_env={} replace_env={}",
        args.len(),
        qol_plugin_daemon::daemon::handed_socket().is_ok(),
        qol_plugin_daemon::daemon::asked_to_replace_existing()
    );
    qol_shot::cli::exit_code(args)
}

fn should_run_daemon(args: &[String]) -> bool {
    should_run_daemon_with_env(
        args.is_empty(),
        qol_plugin_daemon::daemon::asked_to_replace_existing(),
    )
}

fn should_run_daemon_with_env(args_empty: bool, daemon_spawn: bool) -> bool {
    args_empty && daemon_spawn
}

#[cfg(test)]
mod tests {
    use super::should_run_daemon_with_env;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn no_args_run_daemon_only_for_host_daemon_spawn() {
        assert!(should_run_daemon_with_env(true, true));
        assert!(!should_run_daemon_with_env(true, false));
        assert!(!should_run_daemon_with_env(false, true));
    }
}
