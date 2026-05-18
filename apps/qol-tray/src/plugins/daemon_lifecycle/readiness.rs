use crate::plugins::Plugin;
use anyhow::Result;
use std::process::Child;

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
