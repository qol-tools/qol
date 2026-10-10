#![allow(unsafe_code)]

mod attach;
mod close;
mod keys;
mod launch;
mod peb;
mod probe;
mod report;
mod send;
mod spawn;
mod tabs;
mod title;

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use qol_app_icon::ProcessEntry;
use qol_windowing::platform::windows::{top_level_windows, Window};
use qol_windowing::WindowId;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindow, IsWindowVisible, GW_OWNER};

use crate::console::{PROBE_COMMAND, SEND_COMMAND, TITLE_COMMAND};
use crate::{
    BackendId, DeliveryMode, ScreenReader, SessionBinding, SessionCloser, SessionFacts,
    SessionFocus, SessionId, SessionInventory, SessionSpawner, SpawnRequest, SpawnSurface,
    TerminalBackend, TerminalError, TerminalSnapshot, TextInput,
};

use self::close::{belongs, settle, token_start, Member};
use self::keys::Key;
use self::report::{
    candidate_roots, image_stem, listed, parse_reports, session_facts, ConsoleReport, Root,
};

const BACKEND: &str = "console";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const HELPER_TIMEOUT: Duration = Duration::from_secs(5);
const SCREEN_TTL: Duration = Duration::from_secs(2);
const ANCESTOR_LIMIT: usize = 8;
const DESKTOP_SHELL: &str = "explorer";
const CLOSE_GRACE: Duration = Duration::from_secs(2);
const GRACEFUL_EXIT: u32 = 0;

static BACKEND_ID: LazyLock<BackendId> =
    LazyLock::new(|| BackendId::new(BACKEND).expect("console is a valid terminal backend id"));

pub(crate) fn system_backends() -> Vec<Arc<dyn TerminalBackend>> {
    vec![Arc::new(ConsoleBackend::default())]
}

pub(crate) fn console_probe(args: &[String]) -> io::Result<String> {
    probe::run(args)
}

pub(crate) fn console_send(args: &[String]) -> io::Result<String> {
    send::run(args)
}

pub(crate) fn console_launch(args: &[String]) -> io::Result<String> {
    launch::run(args)
}

pub(crate) fn console_title(args: &[String]) -> io::Result<String> {
    title::run(args)
}

pub(crate) fn spawn_backend() -> &'static BackendId {
    &BACKEND_ID
}

struct Probed {
    report: ConsoleReport,
    at: Instant,
}

#[derive(Default)]
struct ConsoleBackend {
    reports: Mutex<HashMap<i32, Probed>>,
    current: OnceLock<SessionId>,
}

impl ConsoleBackend {
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

    fn cached(&self, root: i32) -> Option<ConsoleReport> {
        self.reports.lock().ok().and_then(|cache| {
            cache
                .get(&root)
                .filter(|probed| probed.at.elapsed() < SCREEN_TTL)
                .map(|probed| probed.report.clone())
        })
    }

    fn fresh(&self, target: &SessionBinding) -> Result<ConsoleReport, TerminalError> {
        verify(target)?;
        let root = target.root_pid();
        let reports = probe_consoles(&[root])?;
        self.remember(&reports);
        reports
            .into_iter()
            .find(|report| report.root == root)
            .ok_or_else(|| TerminalError::TargetMissing(target.clone()))
    }

    fn report(&self, target: &SessionBinding) -> Result<ConsoleReport, TerminalError> {
        verify(target)?;
        match self.cached(target.root_pid()) {
            Some(report) => Ok(report),
            None => self.fresh(target),
        }
    }
}

impl TerminalBackend for ConsoleBackend {
    fn read_screen_from_snapshot(
        &self,
        snapshot: &TerminalSnapshot,
        target: &SessionBinding,
    ) -> Result<String, TerminalError> {
        snapshot.validate_screen_target(target)?;
        self.report(target).map(|report| report.screen)
    }

    fn id(&self) -> &BackendId {
        &BACKEND_ID
    }

    fn current_session_id(&self) -> Option<SessionId> {
        if let Some(current) = self.current.get() {
            return Some(current.clone());
        }
        let table = qol_app_icon::processes();
        let own = i32::try_from(std::process::id()).ok()?;
        let roots = candidate_roots(&table, own);
        let chain = console_chain(&table, &roots, own);
        if chain.is_empty() {
            return None;
        }
        let reports = probe_consoles(&chain).ok()?;
        self.remember(&reports);
        let shown: Vec<i32> = listed(&reports, &roots)
            .into_iter()
            .map(|report| report.root)
            .collect();
        let root = chain.into_iter().find(|pid| shown.contains(pid))?;
        let current = SessionId::new(BACKEND_ID.clone(), native_id(root)).ok()?;
        let _ = self.current.set(current.clone());
        Some(current)
    }

    fn spawner(&self) -> Option<&dyn SessionSpawner> {
        Some(self)
    }

    fn closer(&self) -> Option<&dyn SessionCloser> {
        Some(self)
    }
}

impl SessionSpawner for ConsoleBackend {
    fn supports(&self, _surface: SpawnSurface) -> bool {
        true
    }

    fn spawn(&self, request: &SpawnRequest) -> Result<SessionId, TerminalError> {
        let root = spawn::spawn(request)?;
        let session = SessionId::new(BACKEND_ID.clone(), native_id(root)).map_err(|error| {
            TerminalError::SpawnFailed {
                backend: BACKEND_ID.clone(),
                message: error.to_string(),
            }
        })?;
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "backend={} operation=spawn surface={} root={} outcome=ok",
            &*BACKEND_ID,
            request.identity.surface,
            root
        );
        Ok(session)
    }
}

impl SessionCloser for ConsoleBackend {
    fn close(&self, target: &SessionBinding) -> Result<(), TerminalError> {
        let report = self.fresh(target)?;
        let refused = |message: &str| TerminalError::CommandFailed {
            backend: BACKEND_ID.clone(),
            operation: "close session",
            code: None,
            stderr: message.to_owned(),
        };
        let started = token_start(target.session_id().native())
            .ok_or_else(|| refused("the session start time is unknown"))?;
        let root = Member::open(report.root)
            .filter(|root| belongs(started, true, root.start()))
            .ok_or_else(|| TerminalError::TargetMissing(target.clone()))?;
        let own = i32::try_from(std::process::id()).unwrap_or_default();
        let mut members: Vec<Member> = report
            .attached
            .iter()
            .filter(|pid| **pid != report.root && **pid != own)
            .filter_map(|pid| Member::open(*pid))
            .filter(|member| belongs(started, false, member.start()))
            .collect();
        let asked =
            owns_window(&report) && window_from(report.window).is_some_and(Window::request_close);
        let ended = root.terminate(GRACEFUL_EXIT);
        if !asked && !ended {
            return Err(refused("the session root refused to terminate"));
        }
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "backend={} operation=close root={} members={} window_close={}",
            &*BACKEND_ID,
            report.root,
            members.len(),
            asked
        );
        members.push(root);
        settle(&members, CLOSE_GRACE);
        Ok(())
    }
}

impl SessionInventory for ConsoleBackend {
    fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
        let table = qol_app_icon::processes();
        let own = i32::try_from(std::process::id()).unwrap_or_default();
        let roots = candidate_roots(&table, own);
        let reports = if roots.is_empty() {
            Vec::new()
        } else {
            let pids: Vec<i32> = roots.iter().map(|root| root.pid).collect();
            probe_consoles(&pids)?
        };
        self.remember(&reports);
        let sessions: Vec<SessionFacts> = listed(&reports, &roots)
            .into_iter()
            .filter_map(|report| {
                session_facts(&BACKEND_ID, &native_id(report.root), report, &table)
            })
            .collect();
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "backend={} operation=discover roots={} sessions={}",
            &*BACKEND_ID,
            roots.len(),
            sessions.len()
        );
        Ok(sessions)
    }
}

impl ScreenReader for ConsoleBackend {
    fn read_screen(&self, target: &SessionBinding) -> Result<String, TerminalError> {
        self.fresh(target).map(|report| report.screen)
    }

    fn read_screen_relaxed(&self, target: &SessionBinding) -> Result<String, TerminalError> {
        self.report(target).map(|report| report.screen)
    }

    fn read_screen_unscrolled(&self, target: &SessionBinding) -> Result<String, TerminalError> {
        self.report(target)
            .map(|report| report.live_screen.unwrap_or(report.screen))
    }
}

impl SessionFocus for ConsoleBackend {
    fn focus(&self, target: &SessionBinding) -> Result<(), TerminalError> {
        let report = self.report(target)?;
        let focused = focus_window(&report).is_some_and(|window| {
            if !owns_window(&report) {
                select_tab(window, &report);
            }
            window.activate()
        });
        if focused {
            return Ok(());
        }
        Err(TerminalError::CommandFailed {
            backend: BACKEND_ID.clone(),
            operation: "focus session",
            code: None,
            stderr: "no terminal window accepted focus".to_owned(),
        })
    }
}

impl TextInput for ConsoleBackend {
    fn send_text(
        &self,
        target: &SessionBinding,
        text: &str,
        mode: DeliveryMode,
    ) -> Result<(), TerminalError> {
        verify(target)?;
        if text.is_empty() {
            return Ok(());
        }
        let mode = match mode {
            DeliveryMode::Insert => send::INSERT,
            DeliveryMode::Submit => send::SUBMIT,
        };
        let args = [
            SEND_COMMAND.to_owned(),
            target.root_pid().to_string(),
            mode.to_owned(),
        ];
        run_helper("send text", &args, Some(text)).map(drop)
    }

    fn send_key(&self, target: &SessionBinding, key: &str) -> Result<(), TerminalError> {
        verify(target)?;
        if Key::parse(key).is_none() {
            return Err(TerminalError::Unsupported {
                target: target.session_id().clone(),
                capability: "that key",
            });
        }
        let args = [
            SEND_COMMAND.to_owned(),
            target.root_pid().to_string(),
            send::KEY.to_owned(),
            key.to_owned(),
        ];
        run_helper("send key", &args, None).map(drop)
    }
}

fn verify(target: &SessionBinding) -> Result<(), TerminalError> {
    let current = target.session_id().backend() == &*BACKEND_ID
        && native_id(target.root_pid()) == target.session_id().native();
    if current {
        return Ok(());
    }
    Err(TerminalError::TargetMissing(target.clone()))
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

fn owns_window(report: &ConsoleReport) -> bool {
    report.window_visible && !report.window_owned
}

fn focus_window(report: &ConsoleReport) -> Option<Window> {
    let console = report.window as usize as HWND;
    if owns_window(report) {
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

fn select_tab(window: Window, report: &ConsoleReport) -> bool {
    let Some(handle) = window.id().as_u32() else {
        return false;
    };
    let root = report.root;
    let selected = tabs::select_tab(handle, &report.title, move |title| {
        let args = [TITLE_COMMAND.to_owned(), root.to_string()];
        run_helper("set title", &args, Some(title)).is_ok()
    });
    qol_runtime::probe!(
        "TERMINAL_SESSIONS",
        "backend={} operation=select_tab root={} selected={}",
        &*BACKEND_ID,
        root,
        selected
    );
    selected
}

fn console_chain(table: &[ProcessEntry], roots: &[Root], own: i32) -> Vec<i32> {
    ancestors(table, own)
        .into_iter()
        .filter(|pid| roots.iter().any(|root| root.pid == *pid))
        .collect()
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

fn probe_consoles(pids: &[i32]) -> Result<Vec<ConsoleReport>, TerminalError> {
    let args: Vec<String> = std::iter::once(PROBE_COMMAND.to_owned())
        .chain(pids.iter().map(i32::to_string))
        .collect();
    let stdout = run_helper("probe consoles", &args, None)?;
    parse_reports(&stdout).map_err(|source| TerminalError::InvalidResponse {
        backend: BACKEND_ID.clone(),
        source,
    })
}

fn run_helper(
    operation: &'static str,
    args: &[String],
    input: Option<&str>,
) -> Result<String, TerminalError> {
    let unavailable = |source| TerminalError::BackendUnavailable {
        backend: BACKEND_ID.clone(),
        source,
    };
    let executable = std::env::current_exe().map_err(unavailable)?;
    let mut child = Command::new(executable)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(unavailable)?;
    let writer = feed(&mut child, input);
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let status = qol_process::wait_for_exit_or_terminate(&mut child, HELPER_TIMEOUT);
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let stdout = joined(stdout);
    let stderr = joined(stderr);
    let code = status.as_ref().ok().and_then(|status| status.code());
    let success = status.as_ref().is_ok_and(|status| status.success());
    qol_runtime::probe!(
        "TERMINAL_SESSIONS",
        "backend={} operation={} success={} code={:?} stdout_len={}",
        &*BACKEND_ID,
        operation,
        success,
        code,
        stdout.len()
    );
    if !success {
        return Err(TerminalError::CommandFailed {
            backend: BACKEND_ID.clone(),
            operation,
            code,
            stderr: stderr.trim().to_owned(),
        });
    }
    Ok(stdout)
}

fn feed(child: &mut Child, input: Option<&str>) -> Option<JoinHandle<()>> {
    let mut stdin = child.stdin.take()?;
    let text = input?.to_owned();
    Some(std::thread::spawn(move || {
        let _ = stdin.write_all(text.as_bytes());
    }))
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> Option<JoinHandle<String>> {
    let mut pipe = pipe?;
    Some(std::thread::spawn(move || {
        let mut text = String::new();
        let _ = pipe.read_to_string(&mut text);
        text
    }))
}

fn joined(reader: Option<JoinHandle<String>>) -> String {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
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

    #[test]
    fn the_console_chain_lists_ancestor_roots_nearest_first() {
        let table = [
            process(1, 0, "explorer.exe"),
            process(2, 1, "WindowsTerminal.exe"),
            process(3, 2, "pwsh.exe"),
            process(4, 3, "claude.exe"),
            process(5, 4, "cmd.exe"),
            process(6, 5, "conhost.exe"),
            process(7, 5, "qol.exe"),
            process(8, 7, "conhost.exe"),
        ];
        let roots = candidate_roots(&table, 7);
        assert_eq!(console_chain(&table, &roots, 7), vec![5, 3]);
        assert_eq!(console_chain(&table, &roots, 4), vec![3]);
        assert!(console_chain(&table, &roots, 2).is_empty());
    }

    #[test]
    fn bindings_from_another_backend_or_process_start_are_missing() {
        let own = i32::try_from(std::process::id()).unwrap();
        let backend = BackendId::new("kitty").unwrap();
        let cases = [
            (BACKEND_ID.clone(), native_id(own), true),
            (BACKEND_ID.clone(), format!("{own}-1"), false),
            (backend, native_id(own), false),
        ];
        for (backend, native, current) in cases {
            let target =
                SessionBinding::new(crate::SessionId::new(backend, native).unwrap(), own).unwrap();
            assert_eq!(verify(&target).is_ok(), current, "{target}");
        }
    }
}
