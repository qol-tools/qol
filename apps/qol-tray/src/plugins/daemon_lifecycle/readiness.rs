use crate::plugins::Plugin;
use anyhow::Result;
use std::path::Path;
use std::process::Child;

pub(super) fn wait_for_daemon_ready(
    plugin: &Plugin,
    daemon_config: &crate::plugins::manifest::DaemonConfig,
    child: &mut Child,
) -> Result<()> {
    let Some(socket) = daemon_config.socket.as_deref() else {
        return wait_for_non_socket_start(plugin, child);
    };

    wait_for_socket_start(plugin, child, socket)
}

pub(super) fn wait_for_exit(plugin: &Plugin, child: &mut Child) -> Result<()> {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(2);

    loop {
        match child.try_wait()? {
            Some(_) => return Ok(()),
            None if start.elapsed() >= timeout => return force_kill_daemon(plugin, child),
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
}

pub(super) fn terminate_daemon(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }

    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

fn wait_for_socket_start(plugin: &Plugin, child: &mut Child, socket: &str) -> Result<()> {
    if wait_for_socket(socket, child) {
        return Ok(());
    }

    let _ = child.kill();
    let _ = child.wait();
    anyhow::bail!(
        "Daemon for {} failed to bind socket within timeout",
        plugin.id
    )
}

fn wait_for_non_socket_start(plugin: &Plugin, child: &mut Child) -> Result<()> {
    std::thread::sleep(std::time::Duration::from_millis(100));
    let exited = child.try_wait()?;
    let Some(status) = exited else {
        return Ok(());
    };
    if status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "Daemon for {} exited immediately with {}",
        plugin.id,
        status
    )
}

fn force_kill_daemon(plugin: &Plugin, child: &mut Child) -> Result<()> {
    log::warn!(
        "Daemon for {} didn't exit gracefully, forcing kill",
        plugin.id
    );
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    child.kill()?;
    child.wait()?;
    Ok(())
}

fn wait_for_socket(socket_path: &str, child: &mut Child) -> bool {
    let path = Path::new(socket_path);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let poll_interval = std::time::Duration::from_millis(50);

    while std::time::Instant::now() < deadline {
        if daemon_exited(child) {
            return false;
        }
        if socket_reachable(path) {
            return true;
        }
        std::thread::sleep(poll_interval);
    }
    false
}

fn daemon_exited(child: &mut Child) -> bool {
    match child.try_wait() {
        Ok(Some(status)) => {
            log::error!("Daemon exited early with {}", status);
            true
        }
        _ => false,
    }
}

fn socket_reachable(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(path).is_ok()
    }

    #[cfg(not(unix))]
    {
        true
    }
}
