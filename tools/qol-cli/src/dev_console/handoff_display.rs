use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};

use super::{Dash, Reload, TICK};

enum Update {
    Log(String),
    Phase(&'static str, &'static str),
    RunningWorktree(PathBuf),
}

pub(super) struct HandoffUpdates {
    sender: Sender<Update>,
    started: Instant,
}

impl HandoffUpdates {
    pub(super) fn push_log(&self, line: impl Into<String>) {
        let _ = self.sender.send(Update::Log(line.into()));
    }

    pub(super) fn phase(&self, phase: &'static str, detail: &'static str) {
        self.push_log(format!(
            "[qol dev] reload phase={phase} elapsed_ms={} detail={detail}",
            self.started.elapsed().as_millis()
        ));
        let _ = self.sender.send(Update::Phase(phase, detail));
    }

    pub(super) fn adopt_running_worktree(&self, root: PathBuf) {
        let _ = self.sender.send(Update::RunningWorktree(root));
    }
}

pub(super) fn run<T>(
    dash: &mut Dash,
    render: impl FnMut(&mut Dash) -> Result<()> + Send,
    handoff: impl FnOnce(&HandoffUpdates) -> T,
) -> Result<T> {
    let started = match &dash.reload {
        Reload::Handoff { activity } => activity.started,
        Reload::Idle | Reload::Running { .. } => bail!("reload prebuild is not ready for handoff"),
    };
    let result = std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        let updates = HandoffUpdates { sender, started };
        let display = std::thread::Builder::new()
            .name("qol-reload-display".to_string())
            .spawn_scoped(scope, || render_updates(dash, receiver, render))
            .context("failed to start reload display")?;
        let result = handoff(&updates);
        drop(updates);
        display
            .join()
            .map_err(|_| anyhow!("reload display panicked"))??;
        Ok(result)
    });
    dash.reload = Reload::Idle;
    result
}

fn render_updates(
    dash: &mut Dash,
    updates: Receiver<Update>,
    mut render: impl FnMut(&mut Dash) -> Result<()>,
) -> Result<()> {
    render(dash)?;
    let mut last_render = Instant::now();
    loop {
        let changed = match updates.recv_timeout(TICK.saturating_sub(last_render.elapsed())) {
            Ok(update) => apply_update(dash, update),
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        if changed || last_render.elapsed() >= TICK {
            render(dash)?;
            last_render = Instant::now();
        }
    }
}

fn apply_update(dash: &mut Dash, update: Update) -> bool {
    match update {
        Update::Log(line) => {
            dash.push_log(line);
            false
        }
        Update::Phase(phase, detail) => {
            if let Reload::Handoff { activity } = &mut dash.reload {
                activity.phase = phase.to_string();
                activity.detail = detail.to_string();
            }
            true
        }
        Update::RunningWorktree(root) => {
            dash.adopt_running_worktree(root);
            dash.pokes.doctor = true;
            dash.pokes.links = true;
            true
        }
    }
}

#[cfg(test)]
mod tests;
