use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn launch_app(path: &Path, _exec: &[String]) -> bool {
    open_path(path)
}

pub fn open_path(path: &Path) -> bool {
    spawn_null("open", &[path.as_os_str()])
}

pub fn activate_app(_cx: &mut gpui::App) {
    set_activation_policy();
}

pub fn set_activation_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
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
    if let Some(dir) = super::launch_working_dir() {
        command.current_dir(dir);
    }
    command.spawn().is_ok()
}
