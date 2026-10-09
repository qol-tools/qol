use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessId, WaitForSingleObject,
};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::cli::PLUGIN_ID;

use super::catalog::Launch;

const POLL: Duration = Duration::from_millis(250);
const SUCCESS_CODES: &[u32] = &[0, 1605, 1614, 1641, 3010];

struct Process(HANDLE);

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl Process {
    fn exited(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }

    fn exit_code(&self) -> Option<u32> {
        let mut code = 0u32;
        (unsafe { GetExitCodeProcess(self.0, &mut code) } != 0).then_some(code)
    }
}

pub(super) fn run_and_wait(launch: &Launch, elevate: bool, timeout: Duration) -> Result<()> {
    let process = start(launch, elevate)?;
    let root = unsafe { GetProcessId(process.0) };
    let root = i32::try_from(root).unwrap_or(0);
    let started = Instant::now();
    let mut tracked = BTreeMap::new();
    if let Some(start) = qol_app_icon::process_start_time_us(root) {
        tracked.insert(root, start);
    }
    loop {
        let table: Vec<ProcessRow> = qol_app_icon::processes()
            .into_iter()
            .map(|entry| ProcessRow {
                pid: entry.pid,
                parent_pid: entry.parent_pid,
                start: qol_app_icon::process_start_time_us(entry.pid),
            })
            .collect();
        track_descendants(&mut tracked, &table);
        let descendants_alive = tracked
            .iter()
            .any(|(pid, start)| *pid != root && is_alive(&table, *pid, *start));
        if process.exited() && !descendants_alive {
            break;
        }
        if started.elapsed() > timeout {
            bail!("{PLUGIN_ID}: the uninstaller did not finish within {timeout:?}");
        }
        std::thread::sleep(POLL);
    }
    match process.exit_code() {
        Some(code) if SUCCESS_CODES.contains(&code) => Ok(()),
        Some(code) => bail!("{PLUGIN_ID}: the uninstaller exited with code {code}"),
        None => bail!("{PLUGIN_ID}: could not read the uninstaller exit code"),
    }
}

fn start(launch: &Launch, elevate: bool) -> Result<Process> {
    let verb = wide("runas");
    let file = wide(&launch.program);
    let parameters = wide(&launch.arguments);
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    info.hwnd = null_mut();
    info.lpVerb = if elevate {
        verb.as_ptr()
    } else {
        std::ptr::null()
    };
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = SW_SHOWNORMAL;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
            bail!("{PLUGIN_ID}: the administrator prompt was declined");
        }
        bail!("{PLUGIN_ID}: could not start {}: {error}", launch.program);
    }
    if info.hProcess.is_null() {
        bail!("{PLUGIN_ID}: {} did not start a process", launch.program);
    }
    Ok(Process(info.hProcess))
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

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
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
