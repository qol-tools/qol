use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use qol_terminal_sessions::cli::{
    CliSessionDescriptor, CliSessionInterpreter, CliSessionSubscription, CliToolId,
};

use crate::host::{project_of, window_id, Pane, TerminalHost};
use crate::session::git;
use crate::session::registry::{summary_for, Registry, SessionState};
use crate::session::service::ServiceProbe;
use crate::session::status::Status;
use crate::session::tool::Tool;
use crate::signal::screen::screen_hash;
use crate::storage::{paths, persist};
use crate::strategy::{for_tool, running_since_for, status_for, Ctx, Prev, Reading};
use crate::ui::notify::{self, Notice};

const SCREEN_FALLBACK_SECS: u64 = 60;
const SUBSCRIPTION_RETRY_SECS: u64 = 30;

#[derive(Default)]
pub struct ReconcileCaches {
    branch: git::BranchCache,
    persisted: Option<(std::path::PathBuf, Vec<SessionState>)>,
    screens: HashMap<u64, ScreenCache>,
}

struct ScreenCache {
    identity: ScreenIdentity,
    text: Option<String>,
    last_read: u64,
    dirty: Arc<AtomicBool>,
    subscription: Option<CliSessionSubscription>,
    next_subscription_attempt: u64,
}

#[derive(Eq, PartialEq)]
struct ScreenIdentity {
    root_pid: i32,
    foreground_pids: Vec<i32>,
    tool: CliToolId,
    external_id: Option<String>,
}

impl ScreenIdentity {
    fn new(pane: &Pane, cli_session: &CliSessionDescriptor) -> Self {
        Self {
            root_pid: pane.root_pid,
            foreground_pids: pane.foreground_pids.clone(),
            tool: cli_session.tool.id.clone(),
            external_id: cli_session.external_id.clone(),
        }
    }
}

fn pid_alive(pid: i32) -> bool {
    u32::try_from(pid).is_ok_and(qol_process::is_pid_alive)
}

pub fn tick(
    registry: &Arc<Mutex<Registry>>,
    host: &dyn TerminalHost,
    cli_interpreter: &CliSessionInterpreter,
    service_probe: &dyn ServiceProbe,
    now: u64,
) -> Vec<Notice> {
    let mut caches = ReconcileCaches::default();
    tick_with_caches(
        registry,
        host,
        cli_interpreter,
        service_probe,
        now,
        &mut caches,
    )
}

pub fn tick_with_caches(
    registry: &Arc<Mutex<Registry>>,
    host: &dyn TerminalHost,
    cli_interpreter: &CliSessionInterpreter,
    service_probe: &dyn ServiceProbe,
    now: u64,
    caches: &mut ReconcileCaches,
) -> Vec<Notice> {
    let mut notices = Vec::new();
    #[cfg(debug_assertions)]
    let tick_start = std::time::Instant::now();
    let panes = host.discover();
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "CLI_SESSIONS_RECON",
        "phase=tick now={now} panes={}",
        panes.len()
    );
    prune_missing(registry, caches, &panes);

    for pane in &panes {
        let cli_session = cli_interpreter.describe(pane);
        #[cfg(debug_assertions)]
        let cli_tool = cli_session.tool.id.to_string();
        let tool = Tool::from_cli_session(&cli_session);
        let strategy = for_tool(tool);
        let wants_screen = strategy.wants_screen(pane);
        let screen = if wants_screen {
            cached_screen(host, cli_interpreter, pane, &cli_session, now, caches)
        } else {
            caches.screens.remove(&window_id(pane));
            None
        };
        let new_hash = screen.as_deref().map(screen_hash);

        let (prev, prev_hash) = snapshot(registry, window_id(pane));
        let screen_changed = match (new_hash, prev_hash) {
            (Some(n), Some(p)) => n != p,
            (Some(_), None) => true,
            (None, _) => false,
        };

        let is_service = tool == Tool::Generic && !pane.at_prompt && service_probe.is_service(pane);

        let reading = strategy.read(&Ctx {
            pane,
            cli_session,
            screen: screen.as_deref(),
            screen_changed,
            prev,
            now,
            is_service,
        });

        #[cfg(debug_assertions)]
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=pane wid={} tool={:?} cli_tool={} at_prompt={} wants_screen={wants_screen} screen_changed={screen_changed} read_phase={:?} label={:?} title={:?}",
            window_id(pane),
            tool,
            cli_tool,
            pane.at_prompt,
            reading.phase,
            reading.label,
            short(&pane.title)
        );

        let phase = reading.phase;
        let branch = caches.branch.branch(&pane.cwd, now);
        if let Ok(mut reg) = registry.lock() {
            let (notice, status) = apply(&mut reg, pane, tool, reading, new_hash, branch, now);
            if let Some(notice) = notice {
                notices.push(notice);
            }
            drop(reg);
            crate::diagnostics::anomaly::observe(
                window_id(pane),
                now,
                &pane.title,
                screen.as_deref(),
                phase,
                status,
            );
        }
    }

    persist_if_changed(registry, caches);
    #[cfg(debug_assertions)]
    {
        let elapsed_ms = tick_start.elapsed().as_millis();
        if elapsed_ms >= 250 {
            qol_runtime::probe!(
                "CLI_SESSIONS_RECON_SLOW",
                "phase=tick total_ms={elapsed_ms} panes={}",
                panes.len()
            );
        }
    }
    notices
}

fn attention_notice(
    prev: Status,
    new: Status,
    tool: Tool,
    label: Option<&str>,
    cwd: &str,
    summary: &str,
) -> Option<Notice> {
    notify::announces_attention(prev, new).then(|| {
        let name = label.map(str::to_string).unwrap_or_else(|| project_of(cwd));
        Notice::new(tool, name, summary)
    })
}

fn cached_screen(
    host: &dyn TerminalHost,
    cli_interpreter: &CliSessionInterpreter,
    pane: &Pane,
    cli_session: &CliSessionDescriptor,
    now: u64,
    caches: &mut ReconcileCaches,
) -> Option<String> {
    let id = window_id(pane);
    let identity = ScreenIdentity::new(pane, cli_session);
    let replace = caches
        .screens
        .get(&id)
        .is_none_or(|entry| entry.identity != identity);
    if replace {
        caches.screens.insert(
            id,
            ScreenCache {
                identity,
                text: None,
                last_read: 0,
                dirty: Arc::new(AtomicBool::new(true)),
                subscription: None,
                next_subscription_attempt: 0,
            },
        );
    }
    let entry = caches
        .screens
        .get_mut(&id)
        .expect("screen cache was inserted for the current pane");
    if entry.subscription.is_none() && now >= entry.next_subscription_attempt {
        let dirty = entry.dirty.clone();
        entry.subscription = cli_interpreter
            .subscribe(
                pane,
                Arc::new(move || {
                    dirty.store(true, Ordering::Release);
                }),
            )
            .ok()
            .flatten();
        entry.next_subscription_attempt = now.saturating_add(SUBSCRIPTION_RETRY_SECS);
    }
    let signaled = entry.dirty.swap(false, Ordering::AcqRel);
    let reason = if entry.text.is_none() {
        Some("initial")
    } else if entry.subscription.is_none() {
        Some("unsubscribed")
    } else if signaled {
        Some("signal")
    } else if now.saturating_sub(entry.last_read) >= SCREEN_FALLBACK_SECS {
        Some("fallback")
    } else {
        None
    };
    let Some(reason) = reason else {
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=screen wid={id} source=cache age_secs={} subscription=active",
            now.saturating_sub(entry.last_read)
        );
        return entry.text.clone();
    };
    #[cfg(debug_assertions)]
    let started = std::time::Instant::now();
    let fresh = host.get_text(id, pane.root_pid);
    #[cfg(debug_assertions)]
    let elapsed_ms = started.elapsed().as_millis();
    #[cfg(not(debug_assertions))]
    let elapsed_ms = 0_u128;
    if let Some(text) = fresh {
        entry.text = Some(text);
        entry.last_read = now;
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=screen wid={id} source=read reason={reason} elapsed_ms={elapsed_ms} subscription={}",
            if entry.subscription.is_some() {
                "active"
            } else {
                "unsupported"
            }
        );
        return entry.text.clone();
    }
    entry.dirty.store(true, Ordering::Release);
    qol_runtime::probe!(
        "CLI_SESSIONS_RECON",
        "phase=screen wid={id} source=read reason={reason} outcome=unavailable elapsed_ms={elapsed_ms}"
    );
    None
}

fn prune_missing(registry: &Arc<Mutex<Registry>>, caches: &mut ReconcileCaches, panes: &[Pane]) {
    let live: HashSet<u64> = panes.iter().map(window_id).collect();
    caches.screens.retain(|id, _| live.contains(id));
    let Ok(mut reg) = registry.lock() else { return };
    reg.prune(pid_alive);
    let stale: Vec<u64> = reg
        .sorted()
        .into_iter()
        .map(|s| s.window_id)
        .filter(|id| !live.contains(id))
        .collect();
    #[cfg(debug_assertions)]
    if !stale.is_empty() {
        qol_runtime::probe!("CLI_SESSIONS_RECON", "phase=prune removed={:?}", stale);
    }
    for id in stale {
        reg.remove(id);
    }
}

fn persist_if_changed(registry: &Arc<Mutex<Registry>>, caches: &mut ReconcileCaches) {
    let Some(path) = paths::state_path() else {
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=persist outcome=skip reason=no_path"
        );
        return;
    };
    let Ok(reg) = registry.lock() else {
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=persist outcome=skip reason=registry_lock"
        );
        return;
    };
    let sessions = reg.sorted();
    drop(reg);
    if caches
        .persisted
        .as_ref()
        .is_some_and(|(persisted_path, persisted)| {
            persisted_path == &path && persisted == &sessions
        })
    {
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=persist outcome=skip reason=unchanged sessions={}",
            sessions.len()
        );
        return;
    }
    #[cfg(debug_assertions)]
    let session_count = sessions.len();
    #[cfg(not(debug_assertions))]
    let session_count = 0_usize;
    if persist::save(&path, &sessions) {
        caches.persisted = Some((path, sessions));
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=persist outcome=save reason=changed sessions={}",
            session_count
        );
        return;
    }
    qol_runtime::probe!(
        "CLI_SESSIONS_RECON",
        "phase=persist outcome=error reason=write_failed sessions={}",
        session_count
    );
}

#[cfg(debug_assertions)]
fn short(s: &str) -> String {
    s.chars()
        .take(48)
        .map(|c| {
            if c == '"' || c.is_whitespace() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

fn snapshot(registry: &Arc<Mutex<Registry>>, window_id: u64) -> (Option<Prev>, Option<u64>) {
    let Ok(reg) = registry.lock() else {
        return (None, None);
    };
    match reg.get(window_id) {
        Some(s) => (
            Some(Prev {
                status: s.status,
                running_since: s.running_since,
            }),
            s.screen_hash,
        ),
        None => (None, None),
    }
}

fn apply(
    reg: &mut Registry,
    pane: &Pane,
    tool: Tool,
    reading: Reading,
    new_hash: Option<u64>,
    branch: Option<String>,
    now: u64,
) -> (Option<Notice>, Status) {
    let pane_window_id = window_id(pane);
    let (prev_status, prev_running) = match reg.get(pane_window_id) {
        Some(s) => (s.status, s.running_since),
        None => (Status::Unknown, None),
    };
    let status = status_for(prev_status, reading.phase);
    let running_since = running_since_for(prev_running, reading.phase, now);
    let summary = summary_for(status, tool);
    let notice = attention_notice(
        prev_status,
        status,
        tool,
        reading.label.as_deref(),
        &pane.cwd,
        &summary,
    );

    #[cfg(debug_assertions)]
    if status != prev_status {
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=apply wid={} prev={:?} new={:?} tool={:?} summary={:?}",
            pane_window_id,
            prev_status,
            status,
            tool,
            summary
        );
    }

    if let Some(s) = reg.get_mut(pane_window_id) {
        if status != s.status {
            s.last_activity = now;
        }
        s.status = status;
        s.tool = tool;
        s.summary = summary;
        s.root_pid = pane.root_pid;
        s.project = project_of(&pane.cwd);
        s.cwd = pane.cwd.clone();
        s.branch = branch;
        s.name = reading.label;
        s.running_since = running_since;
        if let Some(h) = new_hash {
            s.screen_hash = Some(h);
        }
        return (notice, status);
    }
    reg.upsert(SessionState {
        window_id: pane_window_id,
        root_pid: pane.root_pid,
        project: project_of(&pane.cwd),
        name: reading.label,
        cwd: pane.cwd.clone(),
        branch,
        tool,
        status,
        summary,
        last_activity: now,
        screen_hash: new_hash,
        running_since,
    });
    (notice, status)
}
