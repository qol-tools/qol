use std::io;
use std::path::Path;
use std::process::Command;

pub(crate) fn daemon_action_args(_path: &Path, exec: &[String]) -> Option<(String, String)> {
    super::daemon_exec_args(exec).map(|(target, action)| (target.to_string(), action.to_string()))
}

pub(crate) fn launch_app(_path: &Path, exec: &[String]) -> io::Result<()> {
    let Some((program, args)) = exec.split_first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "application has no executable",
        ));
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .env_remove(qol_conventions::ENV_DAEMON_SOCKET)
        .env_remove(qol_conventions::ENV_INSTALL_ID);
    qol_conventions::scrub_daemon_handoff_env(&mut command);
    if let Some(dir) = qol_platform::launch_working_dir() {
        command.current_dir(dir);
    }
    qol_process::spawn_detached(&mut command)
}
