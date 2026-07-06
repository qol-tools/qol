use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::git;
use crate::host::{project_of, Pane, TerminalHost};
use crate::notify::{self, Notice};
use crate::paths;
use crate::persist;
use crate::registry::{summary_for, Registry, SessionState};
use crate::service::ServiceProbe;
use crate::signal::screen::screen_hash;
use crate::status::Status;
use crate::strategy::codex::CodexStore;
use crate::strategy::{for_tool, running_since_for, status_for, Ctx, Prev, Reading};
use crate::tool::{classify, Tool};

#[derive(Default)]
pub struct ReconcileCaches {
    branch: git::BranchCache,
}

fn pid_alive(pid: i32) -> bool {
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

pub fn tick(
    registry: &Arc<Mutex<Registry>>,
    host: &dyn TerminalHost,
    codex_store: &dyn CodexStore,
    service_probe: &dyn ServiceProbe,
    now: u64,
) -> Vec<Notice> {
    let mut caches = ReconcileCaches::default();
    tick_with_caches(registry, host, codex_store, service_probe, now, &mut caches)
}

pub fn tick_with_caches(
    registry: &Arc<Mutex<Registry>>,
    host: &dyn TerminalHost,
    codex_store: &dyn CodexStore,
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
    prune_missing(registry, &panes);

    for pane in &panes {
        let tool = classify(&pane.foreground_basenames);
        let strategy = for_tool(tool, codex_store);
        let wants_screen = strategy.wants_screen(pane);
        let screen = if wants_screen {
            host.get_text(pane.window_id)
        } else {
            None
        };
        let new_hash = screen.as_deref().map(screen_hash);

        let (prev, prev_hash) = snapshot(registry, pane.window_id);
        let screen_changed = match (new_hash, prev_hash) {
            (Some(n), Some(p)) => n != p,
            (Some(_), None) => true,
            (None, _) => false,
        };

        let is_service = tool == Tool::Generic && !pane.at_prompt && service_probe.is_service(pane);

        let reading = strategy.read(&Ctx {
            pane,
            screen: screen.as_deref(),
            screen_changed,
            prev,
            now,
            is_service,
        });

        #[cfg(debug_assertions)]
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=pane wid={} tool={:?} at_prompt={} wants_screen={wants_screen} screen_changed={screen_changed} read_phase={:?} label={:?} title={:?}",
            pane.window_id,
            tool,
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
            crate::anomaly::observe(
                pane.window_id,
                now,
                &pane.title,
                screen.as_deref(),
                phase,
                status,
            );
        }
    }

    if let Ok(reg) = registry.lock() {
        if let Some(path) = paths::state_path() {
            persist::save(&path, &reg.sorted());
        }
    }
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

fn prune_missing(registry: &Arc<Mutex<Registry>>, panes: &[Pane]) {
    let live: HashSet<u64> = panes.iter().map(|p| p.window_id).collect();
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
    let (prev_status, prev_running) = match reg.get(pane.window_id) {
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
            pane.window_id,
            prev_status,
            status,
            tool,
            summary
        );
    }

    if let Some(s) = reg.get_mut(pane.window_id) {
        if status != s.status || status == Status::Working {
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
        window_id: pane.window_id,
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
