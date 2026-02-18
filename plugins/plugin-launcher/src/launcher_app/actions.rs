use std::ffi::OsStr;
use std::process::{Command, Stdio};

use super::search;

pub fn launch_item(item: &search::ResultItem<'_>) -> bool {
    match item {
        search::ResultItem::App(entry) => launch_app(entry),
        search::ResultItem::File(entry) => open_path_detached(&entry.path),
    }
}

fn launch_app(entry: &crate::providers::apps::AppEntry) -> bool {
    eprintln!("[launch] app: {:?} exec: {:?}", entry.name, entry.exec);
    #[cfg(target_os = "macos")]
    {
        open_path_detached(&entry.path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        spawn_detached(&entry.exec)
    }
}

#[cfg(not(target_os = "macos"))]
fn spawn_detached(exec: &[String]) -> bool {
    let Some((cmd, args)) = exec.split_first() else {
        return false;
    };
    let args_ref: Vec<&OsStr> = args.iter().map(|a| OsStr::new(a)).collect();
    #[cfg(target_os = "linux")]
    return try_spawn_detached(cmd, &args_ref);
    #[cfg(not(target_os = "linux"))]
    spawn_null(cmd, &args_ref)
}

fn spawn_null(cmd: &str, args: &[&OsStr]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

fn open_path_detached(path: &std::path::Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let open = OsStr::new("open");
        let exec = OsStr::new("exec");
        try_spawn_detached("xdg-open", &[path.as_os_str()])
            || try_spawn_detached("gio", &[open, path.as_os_str()])
            || try_spawn_detached("exo-open", &[path.as_os_str()])
            || try_spawn_detached("kioclient5", &[exec, path.as_os_str()])
            || try_spawn_detached("kioclient", &[exec, path.as_os_str()])
    }

    #[cfg(target_os = "macos")]
    {
        spawn_null("open", &[path.as_os_str()])
    }

    #[cfg(target_os = "windows")]
    {
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

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        false
    }
}

#[cfg(target_os = "linux")]
fn try_spawn_detached(cmd: &str, args: &[&OsStr]) -> bool {
    let mut setsid_args: Vec<&OsStr> =
        vec![OsStr::new("-f"), OsStr::new(cmd)];
    setsid_args.extend(args.iter().copied());
    spawn_null("setsid", &setsid_args) || spawn_null(cmd, args)
}
