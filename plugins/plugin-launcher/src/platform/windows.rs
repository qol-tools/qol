use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn launch_app(_path: &Path, exec: &[String]) -> bool {
    let Some((cmd, args)) = exec.split_first() else {
        return false;
    };
    let args_ref: Vec<&OsStr> = args.iter().map(|a| OsStr::new(a)).collect();
    spawn_null(cmd, &args_ref)
}

pub fn open_path(path: &Path) -> bool {
    spawn_null(
        "cmd",
        &[
            OsStr::new("/C"),
            OsStr::new("start"),
            OsStr::new(""),
            path.as_os_str(),
        ],
    )
}

pub fn activate_app(cx: &mut gpui::App) {
    cx.activate(true);
}

pub fn set_activation_policy() {}

fn spawn_null(cmd: &str, args: &[&OsStr]) -> bool {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_remove("QOL_TRAY_DAEMON_SOCKET")
        .env_remove("QOL_TRAY_INSTALL_ID");
    if let Some(dir) = super::launch_working_dir() {
        command.current_dir(dir);
    }
    command.spawn().is_ok()
}
