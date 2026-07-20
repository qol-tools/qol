use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if should_run_daemon(&args) {
        qol_runtime::probe!(
            "SHOT_ENTRY",
            "mode=daemon args={} socket_env={} replace_env={}",
            args.len(),
            std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some(),
            std::env::var_os(qol_conventions::ENV_DAEMON_REPLACE_EXISTING).is_some()
        );
        qol_shot::daemon_app::run();
        return ExitCode::SUCCESS;
    }

    qol_runtime::probe!(
        "SHOT_ENTRY",
        "mode=cli args={} socket_env={} replace_env={}",
        args.len(),
        std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some(),
        std::env::var_os(qol_conventions::ENV_DAEMON_REPLACE_EXISTING).is_some()
    );
    qol_shot::cli::exit_code(args)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn should_run_daemon(args: &[String]) -> bool {
    should_run_daemon_with_env(
        args.is_empty(),
        std::env::var_os(qol_conventions::ENV_DAEMON_REPLACE_EXISTING).is_some(),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn should_run_daemon_with_env(args_empty: bool, daemon_spawn: bool) -> bool {
    args_empty && daemon_spawn
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use super::should_run_daemon_with_env;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn no_args_run_daemon_only_for_host_daemon_spawn() {
        assert!(should_run_daemon_with_env(true, true));
        assert!(!should_run_daemon_with_env(true, false));
        assert!(!should_run_daemon_with_env(false, true));
    }
}
