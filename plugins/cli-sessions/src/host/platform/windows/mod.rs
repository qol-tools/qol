mod peb;
mod probe;
mod report;

use std::collections::HashMap;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use qol_app_icon::ProcessEntry;
use qol_terminal_sessions::{BackendId, SessionBinding};
use qol_windowing::platform::windows::{top_level_windows, Window};
use qol_windowing::WindowId;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindow, IsWindowVisible, GW_OWNER};

use crate::host::{Pane, TerminalHost, CONSOLE_PROBE};

use self::report::{candidate_roots, image_stem, session_facts, ConsoleReport};

const BACKEND: &str = "console";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const SCREEN_TTL: Duration = Duration::from_secs(2);
const ANCESTOR_LIMIT: usize = 8;
const DESKTOP_SHELL: &str = "explorer";

static BACKEND_ID: LazyLock<BackendId> =
    LazyLock::new(|| BackendId::new(BACKEND).expect("console is a valid terminal backend id"));

pub(in crate::host) fn system() -> Arc<dyn TerminalHost + Send + Sync> {
    Arc::new(Console::default())
}

pub(in crate::host) fn console_probe(args: &[String]) -> anyhow::Result<String> {
    probe::run(args)
}

struct Probed {
    report: ConsoleReport,
    at: Instant,
}

#[derive(Default)]
struct Console {
    reports: Mutex<HashMap<i32, Probed>>,
}

impl Console {
    fn remember(&self, reports: &[ConsoleReport]) {
        let Ok(mut cache) = self.reports.lock() else {
            return;
        };
        let now = Instant::now();
        for report in reports {
            cache.insert(
                report.root,
                Probed {
                    report: report.clone(),
                    at: now,
                },
            );
        }
        cache.retain(|_, probed| now.duration_since(probed.at) < SCREEN_TTL * 30);
    }

    fn report(&self, target: &SessionBinding) -> Option<ConsoleReport> {
        let root = target.root_pid();
        let cached = self.reports.lock().ok().and_then(|cache| {
            cache
                .get(&root)
                .filter(|probed| probed.at.elapsed() < SCREEN_TTL)
                .map(|probed| probed.report.clone())
        });
        if cached.is_some() {
            return cached;
        }
        let fresh = run_probe(&[root]);
        self.remember(&fresh);
        fresh.into_iter().find(|report| report.root == root)
    }
}

impl TerminalHost for Console {
    fn discover(&self) -> Vec<Pane> {
        let table = qol_app_icon::processes();
        let own = i32::try_from(std::process::id()).unwrap_or_default();
        let roots = candidate_roots(&table, own);
        if roots.is_empty() {
            return Vec::new();
        }
        let pids: Vec<i32> = roots.iter().map(|root| root.pid).collect();
        let reports = run_probe(&pids);
        self.remember(&reports);
        let panes: Vec<Pane> = reports
            .iter()
            .filter(|report| {
                report.window_visible
                    || roots
                        .iter()
                        .any(|root| root.pid == report.root && root.hosted)
            })
            .filter_map(|report| {
                let native = native_id(report.root);
                session_facts(&BACKEND_ID, &native, report, &table)
            })
            .collect();
        qol_runtime::probe!(
            "CLI_SESSIONS_DISCOVER",
            "backend=console roots={} sessions={}",
            roots.len(),
            panes.len()
        );
        panes
    }

    fn get_text(&self, target: &SessionBinding) -> Option<String> {
        self.report(target).map(|report| report.screen)
    }

    fn visual_screen_is_current(&self, target: &SessionBinding, _screen: &str) -> Option<bool> {
        self.report(target).map(|report| report.viewport_current)
    }

    fn focus(&self, target: &SessionBinding) -> anyhow::Result<()> {
        if target.session_id().backend() != &*BACKEND_ID {
            bail!("{} is not a Windows console session", target.session_id());
        }
        let report = self
            .report(target)
            .with_context(|| format!("console session {} is gone", target.session_id()))?;
        let window = focus_window(&report).with_context(|| {
            format!(
                "no terminal window found for console session {}",
                target.session_id()
            )
        })?;
        if window.activate() {
            Ok(())
        } else {
            bail!("Windows refused to focus the terminal window")
        }
    }
}

fn native_id(root: i32) -> String {
    match qol_app_icon::process_start_time_us(root) {
        Some(start) => format!("{root}-{start}"),
        None => root.to_string(),
    }
}

fn window_from(handle: u64) -> Option<Window> {
    u32::try_from(handle)
        .ok()
        .filter(|handle| *handle != 0)
        .and_then(|handle| Window::from_id(&WindowId::from_u32(handle)))
}

fn focus_window(report: &ConsoleReport) -> Option<Window> {
    let console = report.window as usize as HWND;
    if report.window_visible {
        return window_from(report.window);
    }
    if !console.is_null() {
        let owner = unsafe { GetWindow(console, GW_OWNER) };
        if !owner.is_null() && unsafe { IsWindowVisible(owner) } != 0 {
            return window_from(owner as usize as u64);
        }
    }
    let table = qol_app_icon::processes();
    let windows = top_level_windows();
    ancestors(&table, report.root).into_iter().find_map(|pid| {
        windows.iter().copied().find(|window| {
            window.is_switchable()
                && window
                    .pid()
                    .is_some_and(|owner| i32::try_from(owner).ok() == Some(pid))
        })
    })
}

fn ancestors(table: &[ProcessEntry], pid: i32) -> Vec<i32> {
    let parents: HashMap<i32, (i32, String)> = table
        .iter()
        .map(|process| (process.pid, (process.parent_pid, image_stem(&process.name))))
        .collect();
    let mut chain = Vec::new();
    let mut current = pid;
    while chain.len() < ANCESTOR_LIMIT {
        let Some((parent, _)) = parents.get(&current) else {
            break;
        };
        let Some((_, name)) = parents.get(parent) else {
            break;
        };
        if name == DESKTOP_SHELL {
            break;
        }
        chain.push(*parent);
        current = *parent;
    }
    chain
}

fn run_probe(pids: &[i32]) -> Vec<ConsoleReport> {
    let Ok(executable) = std::env::current_exe() else {
        return Vec::new();
    };
    let mut command = Command::new(executable);
    command
        .arg(CONSOLE_PROBE)
        .args(pids.iter().map(i32::to_string))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    let Ok(mut child) = command.spawn() else {
        return Vec::new();
    };
    let Some(mut stdout) = child.stdout.take() else {
        return Vec::new();
    };
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let finished = qol_process::wait_for_exit_or_terminate(&mut child, PROBE_TIMEOUT)
        .is_ok_and(|status| status.success());
    let text = reader.join().unwrap_or_default();
    if !finished {
        qol_runtime::probe!(
            "CLI_SESSIONS_DISCOVER",
            "backend=console outcome=probe_failed"
        );
        return Vec::new();
    }
    serde_json::from_str(text.trim()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: i32, parent_pid: i32, name: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent_pid,
            name: name.to_string(),
        }
    }

    #[test]
    fn ancestors_stop_at_the_desktop_shell() {
        let table = [
            process(1, 0, "explorer.exe"),
            process(2, 1, "WindowsTerminal.exe"),
            process(3, 2, "pwsh.exe"),
            process(4, 3, "claude.exe"),
            process(9, 1, "cmd.exe"),
        ];
        let cases = [(4, vec![3, 2]), (9, vec![]), (77, vec![])];
        for (pid, expected) in cases {
            assert_eq!(ancestors(&table, pid), expected, "pid {pid}");
        }
    }
}
