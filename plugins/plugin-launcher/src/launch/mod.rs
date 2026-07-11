#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("launch platform implementation is required for this target OS");

use std::path::Path;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::process::Command;

use crate::discovery::search;

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn launch_app(_path: &Path, exec: &[String]) -> bool {
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

#[cfg(target_os = "macos")]
pub fn launch_app(path: &Path, _: &[String]) -> bool {
    open_path(path)
}

pub fn open_path(path: &Path) -> bool {
    qol_apps::desktop_integration::open_with_default_app(path).is_ok()
}

pub fn launch_item(item: &search::ResultItem<'_>) -> bool {
    match item {
        search::ResultItem::App(entry) => {
            eprintln!("[launch] app: {:?} exec: {:?}", entry.name, entry.exec);
            launch_app(&entry.path, &entry.exec)
        }
        search::ResultItem::File(entry) => open_path(&entry.path),
    }
}
