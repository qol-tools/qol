use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use qol_platform::native::shell::{ShellProcess, ShellSpawn};
use windows_sys::Win32::Foundation::ERROR_CANCELLED;

use crate::cli::PLUGIN_ID;

use crate::core::platform::uninstall_wait::{step, Limits, Observation, Step};
use crate::core::platform::windows_paths::{same_path, Launch};

const POLL: Duration = Duration::from_millis(250);
const SUCCESS_CODES: &[u32] = &[0, 1605, 1614, 1641, 3010];

pub(super) fn run_and_wait(
    launch: &Launch,
    elevate: bool,
    limits: &Limits,
    registered: impl Fn() -> bool,
) -> Result<bool> {
    let process = start(launch, elevate)?;
    let root_pid = process.id();
    let root = i32::try_from(root_pid).unwrap_or(0);
    let started = Instant::now();
    let mut tracked = BTreeMap::new();
    if let Some(start) = qol_process::process_start_time(root_pid) {
        tracked.insert(root, start);
    }
    let mut exited_at: Option<Instant> = None;
    let mut unregistered_at: Option<Instant> = None;
    loop {
        let entries = qol_process::processes().unwrap_or_default();
        let table: Vec<ProcessRow> = entries
            .iter()
            .filter_map(|entry| {
                let pid = i32::try_from(entry.pid).ok()?;
                Some(ProcessRow {
                    pid,
                    parent_pid: i32::try_from(entry.parent).ok()?,
                    start: qol_process::process_start_time(entry.pid),
                })
            })
            .collect();
        track_descendants(&mut tracked, &table);
        let descendants_alive = tracked
            .iter()
            .any(|(pid, start)| *pid != root && is_alive(&table, *pid, *start));
        let exit_code = process.wait_timeout(Duration::ZERO).map_err(|error| {
            anyhow!("{PLUGIN_ID}: could not read the uninstaller exit code: {error}")
        })?;
        let uninstaller_running = exit_code.is_none() || descendants_alive;
        if uninstaller_running {
            exited_at = None;
        } else if let Some(code) = exit_code.filter(|_| exited_at.is_none()) {
            check_exit_code(code)?;
            exited_at = Some(Instant::now());
        }
        let still_registered = registered();
        if still_registered {
            unregistered_at = None;
        } else if unregistered_at.is_none() {
            unregistered_at = Some(Instant::now());
        }
        let seen = Observation {
            elapsed: started.elapsed(),
            uninstaller_running,
            handler_running: !uninstaller_running && handler_running(launch, &entries),
            registered: still_registered,
            exited_for: exited_at.map(|at| at.elapsed()),
            unregistered_for: unregistered_at.map(|at| at.elapsed()),
        };
        match step(&seen, limits) {
            Step::Wait => std::thread::sleep(POLL),
            Step::Finished => return Ok(true),
            Step::StillInstalled => return Ok(false),
            Step::TimedOut => bail!(
                "{PLUGIN_ID}: the uninstaller did not finish within {:?}",
                limits.overall
            ),
        }
    }
}

fn check_exit_code(code: u32) -> Result<()> {
    if SUCCESS_CODES.contains(&code) {
        return Ok(());
    }
    bail!("{PLUGIN_ID}: the uninstaller exited with code {code}")
}

fn handler_running(launch: &Launch, entries: &[qol_process::ProcessEntry]) -> bool {
    let program = Path::new(&launch.program);
    let Some(name) = program.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    entries
        .iter()
        .filter(|entry| entry.exe.eq_ignore_ascii_case(name))
        .any(|entry| {
            qol_process::process_image_path(entry.pid).is_ok_and(|image| same_path(&image, program))
        })
}

fn start(launch: &Launch, elevate: bool) -> Result<ShellProcess> {
    let shown = ShellSpawn {
        elevate,
        visible: true,
    };
    qol_platform::native::shell::shell_spawn(&launch.program, &launch.arguments, shown).map_err(
        |error| {
            if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
                return anyhow!("{PLUGIN_ID}: the administrator prompt was declined");
            }
            anyhow!("{PLUGIN_ID}: could not start {}: {error}", launch.program)
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProcessRow {
    pub(super) pid: i32,
    pub(super) parent_pid: i32,
    pub(super) start: Option<u64>,
}

pub(super) fn track_descendants(tracked: &mut BTreeMap<i32, u64>, table: &[ProcessRow]) {
    loop {
        let found: Vec<(i32, u64)> = table
            .iter()
            .filter(|row| !tracked.contains_key(&row.pid))
            .filter_map(|row| {
                let start = row.start?;
                let parent_start = *tracked.get(&row.parent_pid)?;
                (start >= parent_start).then_some((row.pid, start))
            })
            .collect();
        if found.is_empty() {
            return;
        }
        tracked.extend(found);
    }
}

fn is_alive(table: &[ProcessRow], pid: i32, start: u64) -> bool {
    table
        .iter()
        .any(|row| row.pid == pid && row.start == Some(start))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: i32, parent_pid: i32, start: u64) -> ProcessRow {
        ProcessRow {
            pid,
            parent_pid,
            start: Some(start),
        }
    }

    #[test]
    fn descendants_are_tracked_transitively_without_reused_parent_ids() {
        let table = [
            row(10, 1, 100),
            row(11, 10, 110),
            row(12, 11, 120),
            row(13, 10, 50),
            row(14, 99, 130),
        ];
        let mut tracked = BTreeMap::from([(10, 100)]);
        track_descendants(&mut tracked, &table);
        assert_eq!(tracked.keys().copied().collect::<Vec<_>>(), [10, 11, 12]);
        assert!(is_alive(&table, 11, 110));
        assert!(!is_alive(&table, 11, 999));
    }
}
