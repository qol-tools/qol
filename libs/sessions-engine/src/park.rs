use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

/// CLI Sessions owns parking; these subcommands forward to its installed binary,
/// so the park runner and the woken-tab close always come from the plugin qol-tray updates.
const PLUGIN_ID: &str = "qol-cli-sessions";
const WATCH_READY_TIMEOUT: Duration = Duration::from_secs(5);
const WATCH_READY_POLL: Duration = Duration::from_millis(50);

pub(super) fn forward(subcommand: &str, rest: &[OsString]) -> Result<()> {
    let binary = plugin_binary()?;
    let status = Command::new(&binary)
        .arg(subcommand)
        .args(rest)
        .status()
        .with_context(|| format!("failed to run {}", binary.display()))?;
    if !status.success() {
        bail!("`{PLUGIN_ID} {subcommand}` failed ({status})");
    }
    Ok(())
}

pub(super) fn watch_fork(
    session: &str,
    model: &str,
    effort: Option<&str>,
    title: &str,
    ready: &Path,
) -> Result<()> {
    start_watcher(&watch_args(session, model, effort, title, ready))?;
    let deadline = Instant::now() + WATCH_READY_TIMEOUT;
    while !ready.exists() {
        if Instant::now() >= deadline {
            bail!("the watcher did not confirm it started, so the installed {PLUGIN_ID} may predate fork watching");
        }
        thread::sleep(WATCH_READY_POLL);
    }
    let _ = std::fs::remove_file(ready);
    Ok(())
}

fn watch_args(
    session: &str,
    model: &str,
    effort: Option<&str>,
    title: &str,
    ready: &Path,
) -> Vec<String> {
    let ready = ready.display().to_string();
    let mut args = [
        "park", "--watch", session, "--ready", &ready, "--model", model, "--title", title,
    ]
    .map(str::to_owned)
    .to_vec();
    if let Some(effort) = effort {
        args.extend(["--effort".to_owned(), effort.to_owned()]);
    }
    args
}

#[cfg(not(test))]
fn start_watcher(args: &[String]) -> Result<()> {
    let program = match std::env::current_exe() {
        Ok(current) if is_plugin_binary(&current) => current,
        _ => plugin_binary()?,
    };
    qol_process::spawn_detached(Command::new(program).args(args))
        .context("failed to start the CLI Sessions fork watcher")
}

#[cfg(test)]
thread_local! {
    pub(super) static STARTED_WATCHERS: std::cell::RefCell<Vec<Vec<String>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    pub(super) static WATCHERS_START: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

#[cfg(test)]
fn start_watcher(args: &[String]) -> Result<()> {
    STARTED_WATCHERS.with(|started| started.borrow_mut().push(args.to_vec()));
    if !WATCHERS_START.with(std::cell::Cell::get) {
        bail!("the watcher exited at startup");
    }
    let ready = args
        .iter()
        .skip_while(|arg| *arg != "--ready")
        .nth(1)
        .context("the watcher needs --ready")?;
    std::fs::write(ready, "")?;
    Ok(())
}

pub(super) fn is_plugin_binary(path: &Path) -> bool {
    path.file_stem().is_some_and(|stem| stem == PLUGIN_ID)
}

pub(super) fn plugin_binary() -> Result<PathBuf> {
    let binary = qol_config::config_dir()
        .context("cannot resolve the qol-tray config directory")?
        .join("plugins")
        .join(PLUGIN_ID)
        .join(format!("{PLUGIN_ID}{}", std::env::consts::EXE_SUFFIX));
    if !binary.is_file() {
        bail!(
            "CLI Sessions is not installed: {} is missing",
            binary.display()
        );
    }
    Ok(binary)
}
