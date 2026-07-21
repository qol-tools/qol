use std::path::Path;
use std::process::Command;

pub(crate) fn launch_app(_path: &Path, exec: &[String]) -> bool {
    let Some((program, args)) = exec.split_first() else {
        return false;
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .env_remove(qol_conventions::ENV_DAEMON_SOCKET)
        .env_remove(qol_conventions::ENV_INSTALL_ID);
    if let Some(dir) = qol_platform::launch_working_dir() {
        command.current_dir(dir);
    }
    qol_process::spawn_detached(&mut command).is_ok()
}
