use std::process::{Command, Stdio};

use super::search;

pub fn launch_item(item: &search::ResultItem<'_>) -> bool {
    match item {
        search::ResultItem::App(entry) => {
            eprintln!("[launch] app name={:?} path={:?} exec={:?}", entry.name, entry.path, entry.exec);
            launch_app(entry)
        }
        search::ResultItem::File(entry) => {
            eprintln!("[launch] file name={:?} path={:?}", entry.name, entry.path);
            open_path_detached(&entry.path)
        }
    }
}

fn launch_app(entry: &crate::providers::apps::AppEntry) -> bool {
    #[cfg(target_os = "macos")]
    {
        let result = open_path_detached(&entry.path);
        eprintln!("[launch] open_path_detached result={}", result);
        return result;
    }
    #[cfg(not(target_os = "macos"))]
    {
        spawn_detached(&entry.exec)
    }
}

#[cfg(target_os = "linux")]
fn spawn_detached(exec: &str) -> bool {
    let mut parts = exec.split_whitespace();
    let Some(cmd) = parts.next() else { return false };
    let args: Vec<String> = parts.map(ToString::to_string).collect();
    let args_ref: Vec<&std::ffi::OsStr> = args.iter().map(|a| std::ffi::OsStr::new(a)).collect();
    try_spawn_detached(cmd, &args_ref)
}

fn open_path_detached(path: &std::path::Path) -> bool {
    eprintln!("[launch] open_path_detached path={:?}", path);

    #[cfg(target_os = "linux")]
    {
        if try_spawn_detached("xdg-open", &[path.as_os_str()]) {
            return true;
        }
        if try_spawn_detached("gio", &[std::ffi::OsStr::new("open"), path.as_os_str()]) {
            return true;
        }
        if try_spawn_detached("exo-open", &[path.as_os_str()]) {
            return true;
        }
        if try_spawn_detached("kioclient5", &[std::ffi::OsStr::new("exec"), path.as_os_str()]) {
            return true;
        }
        return try_spawn_detached("kioclient", &[std::ffi::OsStr::new("exec"), path.as_os_str()]);
    }

    #[cfg(target_os = "macos")]
    {
        let result = Command::new("open")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match &result {
            Ok(child) => eprintln!("[launch] spawned open pid={}", child.id()),
            Err(e) => eprintln!("[launch] failed to spawn open: {}", e),
        }
        return result.is_ok();
    }

    #[cfg(target_os = "windows")]
    {
        return Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok();
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

#[cfg(target_os = "linux")]
fn try_spawn_detached(cmd: &str, args: &[&std::ffi::OsStr]) -> bool {
    if Command::new("setsid")
        .arg("-f")
        .arg(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
    {
        return true;
    }

    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}
