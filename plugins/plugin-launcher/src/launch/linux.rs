use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn launch_app(_path: &Path, exec: &[String]) -> bool {
    let Some((cmd, args)) = exec.split_first() else {
        return false;
    };
    let args_ref: Vec<&OsStr> = args.iter().map(|a| OsStr::new(a)).collect();
    try_spawn_detached(cmd, &args_ref)
}

pub fn open_path(path: &Path) -> bool {
    let open = OsStr::new("open");
    let exec = OsStr::new("exec");
    try_spawn_detached("xdg-open", &[path.as_os_str()])
        || try_spawn_detached("gio", &[open, path.as_os_str()])
        || try_spawn_detached("exo-open", &[path.as_os_str()])
        || try_spawn_detached("kioclient5", &[exec, path.as_os_str()])
        || try_spawn_detached("kioclient", &[exec, path.as_os_str()])
}

fn try_spawn_detached(cmd: &str, args: &[&OsStr]) -> bool {
    let mut setsid_args: Vec<&OsStr> = vec![OsStr::new("-f"), OsStr::new(cmd)];
    setsid_args.extend(args.iter().copied());
    spawn_null("setsid", &setsid_args) || spawn_null(cmd, args)
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
