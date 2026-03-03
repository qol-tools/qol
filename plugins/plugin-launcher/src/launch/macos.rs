use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn launch_app(path: &Path, _exec: &[String]) -> bool {
    open_path(path)
}

pub fn open_path(path: &Path) -> bool {
    spawn_null("open", &[path.as_os_str()])
}

fn spawn_null(cmd: &str, args: &[&OsStr]) -> bool {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_remove("QOL_TRAY_DAEMON_SOCKET")
        .env_remove("QOL_TRAY_INSTALL_ID");
    if let Some(dir) = qol_plugin_api::platform::launch_working_dir() {
        command.current_dir(dir);
    }
    command.spawn().is_ok()
}
