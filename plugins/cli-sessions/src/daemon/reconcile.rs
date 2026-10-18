use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use qol_sessions_engine::attention::{AttentionRow, AttentionSnapshot, AttentionState};
use qol_terminal_sessions::bridge::LiveBridgeReader;
use qol_terminal_sessions::cli::generic_tool;
use qol_terminal_sessions::cli::{
    CliRuntimeState, CliSessionDescriptor, CliSessionInterpreter, CliSessionSubscription,
    CliToolId, CliViewportState,
};
use qol_terminal_sessions::park::ParkRecord;
use qol_terminal_sessions::{SessionBinding, SessionId};

use super::knocks;
use super::screen_analysis::ScreenAnalysis;
use crate::attention::{reduce_with_policy, Attention, Evidence, Reason, Reduction};
use crate::host::{project_of, Pane, TerminalHost};
use crate::session::git;
use crate::session::registry::{
    is_master_key, master_ready, meaningful_name, summary_for, Registry, SessionState,
};
use crate::session::service::ServiceProbe;
use crate::session::status::Status;
use crate::session::tool::{completion_policy, from_cli_session, is_generic, Tool};
use crate::storage::{paths, persist};
use crate::ui::notify::{self, Notice};

const SCREEN_FALLBACK_SECS: u64 = 60;
const SUBSCRIPTION_RETRY_SECS: u64 = 30;
const ATTENTION_HEARTBEAT_SECS: u64 = 10;

#[derive(Default)]
pub struct ReconcileCaches {
    branch: git::BranchCache,
    persisted: Option<(std::path::PathBuf, Vec<SessionState>)>,
    screens: HashMap<SessionId, ScreenCache>,
    bridges: LiveBridgeReader,
    parked_dir: Option<std::path::PathBuf>,
    messaging: Option<Messaging>,
}

impl ReconcileCaches {
    pub fn with_parked_dir(parked_dir: Option<std::path::PathBuf>) -> Self {
        Self {
            parked_dir,
            ..Self::default()
        }
    }

    pub fn with_messaging(mut self) -> Self {
        self.messaging = Some(Messaging {
            courier: knocks::Courier::system(),
            attention: None,
        });
        self
    }
}

struct Messaging {
    courier: knocks::Courier,
    attention: Option<(Vec<AttentionRow>, u64)>,
}

impl Messaging {
    fn tick(
        &mut self,
        registry: &Arc<Mutex<Registry>>,
        cli_interpreter: &CliSessionInterpreter,
        panes: &[Pane],
        wall_now: u64,
    ) {
        let Ok(keys) = registry.lock().map(|reg| reg.keys()) else {
            return;
        };
        let counts = keys
            .into_iter()
            .map(|key| {
                let counts = qol_sessions_engine::mail_counts(&key);
                (key, counts)
            })
            .collect();
        let Ok(mut reg) = registry.lock() else { return };
        reg.set_mail(&counts);
        let rows = attention_rows(&reg);
        let recipients = knocks::recipients(panes, &reg);
        drop(reg);
        if attention_due(self.attention.as_ref(), &rows, wall_now) {
            let snapshot = AttentionSnapshot {
                written_at_ms: wall_now.saturating_mul(1000),
                sessions: rows,
            };
            match qol_sessions_engine::attention::write(&snapshot) {
                Ok(()) => self.attention = Some((snapshot.sessions, wall_now)),
                Err(error) => log::warn!("[cli-sessions] attention snapshot not written: {error}"),
            }
        }
        self.courier.deliver(cli_interpreter, &recipients, panes);
    }
}

fn attention_due(
    written: Option<&(Vec<AttentionRow>, u64)>,
    rows: &[AttentionRow],
    wall_now: u64,
) -> bool {
    written.is_none_or(|(written_rows, written_at)| {
        written_rows != rows || wall_now.saturating_sub(*written_at) >= ATTENTION_HEARTBEAT_SECS
    })
}

fn attention_rows(reg: &Registry) -> Vec<AttentionRow> {
    reg.sorted()
        .into_iter()
        .map(|state| AttentionRow {
            title: state.name.unwrap_or(state.project),
            state: attention_state(state.status, state.master),
            key: state.key,
            master: state.master,
        })
        .collect()
}

fn attention_state(status: Status, master: bool) -> AttentionState {
    match status {
        Status::NeedsYou => AttentionState::NeedsYou,
        Status::YourTurn | Status::Acknowledged | Status::Unknown if master => {
            AttentionState::Ready
        }
        Status::YourTurn | Status::Acknowledged => AttentionState::YourTurn,
        Status::Working | Status::Service | Status::Coordinating | Status::AwaitingReview => {
            AttentionState::Working
        }
        Status::Parked => AttentionState::Parked,
        Status::Unknown => AttentionState::Idle,
    }
}

struct ScreenCache {
    identity: ScreenIdentity,
    analysis: Option<Arc<ScreenAnalysis>>,
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
    wall_now: u64,
    mono_now: u64,
) -> Vec<Notice> {
    let mut caches = ReconcileCaches::default();
    tick_with_caches(
        registry,
        host,
        cli_interpreter,
        service_probe,
        wall_now,
        mono_now,
        &mut caches,
    )
}

pub fn tick_with_caches(
    registry: &Arc<Mutex<Registry>>,
    host: &dyn TerminalHost,
    cli_interpreter: &CliSessionInterpreter,
    service_probe: &dyn ServiceProbe,
    wall_now: u64,
    mono_now: u64,
    caches: &mut ReconcileCaches,
) -> Vec<Notice> {
    let mut notices = Vec::new();
    #[cfg(debug_assertions)]
    let tick_start = std::time::Instant::now();
    let focus_read = registry.lock().ok().map(|reg| reg.focus_read());
    let panes = host.discover();
    if let (Some(read), Ok(mut reg)) = (focus_read, registry.lock()) {
        reg.observe_focus(read, &panes);
    }
    let bridges = live_bridge_sessions(&mut caches.bridges);
    let parked = live_parked_sessions(caches.parked_dir.as_deref());
    let closing = closing_sessions(caches.parked_dir.as_deref());
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "CLI_SESSIONS_RECON",
        "phase=tick mono_now={mono_now} panes={}",
        panes.len()
    );
    prune_missing(registry, caches, &panes);
    let mut open_conversations = HashSet::new();

    for pane in &panes {
        let cli_session = cli_interpreter.describe(pane);
        open_conversations.extend(cli_session.external_id.clone());
        #[cfg(debug_assertions)]
        let cli_tool = cli_session.tool.id.to_string();
        let tool = from_cli_session(&cli_session);
        let wants_screen = !pane.at_prompt;
        let (prev, prev_hash) = snapshot(registry, &pane.id);
        let refresh_active = matches!(prev.status, Status::Working | Status::NeedsYou);
        let screen = if wants_screen {
            cached_screen(
                host,
                cli_interpreter,
                pane,
                &cli_session,
                refresh_active,
                wall_now,
                caches,
            )
        } else {
            caches.screens.remove(&pane.id);
            None
        };
        let new_hash = screen.as_ref().map(|screen| screen.hash);

        let screen_changed = match (new_hash, prev_hash) {
            (Some(new_hash), Some(prev_hash)) => new_hash != prev_hash,
            (Some(_), None) => true,
            (None, _) => false,
        };

        let is_service = is_generic(&tool) && !pane.at_prompt && service_probe.is_service(pane);
        let screen_evidence = screen
            .as_deref()
            .map(|screen| screen.evidence)
            .unwrap_or_default();
        let evidence = Evidence {
            descriptor_runtime: cli_session.evidence.runtime,
            screen_runtime: screen_evidence.runtime,
            viewport: screen_evidence.viewport,
            file_fresh: cli_session.evidence.activity.file_fresh,
            file_quiet_secs: cli_session.evidence.activity.file_quiet_secs,
            screen_changed,
            at_prompt: pane.at_prompt,
            is_generic: is_generic(&tool),
            is_service,
        };

        let binding_token = SessionBinding::new(pane.id.clone(), pane.root_pid)
            .ok()
            .map(|binding| binding.token());
        let is_bridged = binding_token
            .as_ref()
            .is_some_and(|token| bridges.driven.contains(token));
        let is_parked = binding_token
            .as_ref()
            .is_some_and(|token| parked.iter().any(|park| &park.record.session == token));
        let is_closing = binding_token
            .as_ref()
            .is_some_and(|token| closing.contains(token));
        let driving: Vec<SessionId> = binding_token
            .as_ref()
            .and_then(|token| bridges.driving.get(token))
            .map(|tokens| {
                tokens
                    .iter()
                    .filter_map(|token| token.parse::<SessionBinding>().ok())
                    .map(|binding| binding.session_id().clone())
                    .collect()
            })
            .unwrap_or_default();
        let reduction = reduce_with_policy(&prev, &evidence, mono_now, completion_policy(&tool));
        #[cfg(debug_assertions)]
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=pane id={} tool={:?} cli_tool={} at_prompt={} wants_screen={wants_screen} screen_changed={screen_changed} bridged={is_bridged} driving={} descriptor_runtime={:?} screen_runtime={:?} viewport={:?} fresh={:?} quiet={:?} completion_policy={:?} label={:?} title={:?} reduction_phase={:?} transition={:?}",
            pane.id,
            tool,
            cli_tool,
            pane.at_prompt,
            driving.len(),
            evidence.descriptor_runtime,
            evidence.screen_runtime,
            evidence.viewport,
            evidence.file_fresh,
            evidence.file_quiet_secs,
            completion_policy(&tool),
            cli_session.display_name,
            short(&pane.title),
            reduction.phase,
            reduction.transition.map(|transition| transition.reason)
        );
        let branch = caches.branch.branch(&pane.cwd, wall_now);
        if let Ok(mut reg) = registry.lock() {
            let (notice, status) = apply(
                &mut reg,
                ApplyInput {
                    pane,
                    tool,
                    label: cli_session.display_name.as_deref(),
                    reduction,
                    evidence: &evidence,
                    new_hash,
                    branch,
                    now: mono_now,
                    wall_now,
                    bridged: is_bridged,
                    driving,
                    parked: is_parked,
                    closing: is_closing,
                },
            );
            if let Some(notice) = notice {
                notices.push(notice);
            }
            drop(reg);
            crate::diagnostics::anomaly::observe(
                pane.id.clone(),
                wall_now,
                &pane.title,
                screen.as_ref().map(|screen| screen.text.as_str()),
                reduction.phase,
                status,
            );
        }
    }

    show_detached_parks(
        registry,
        cli_interpreter,
        caches,
        &panes,
        &parked,
        &open_conversations,
        wall_now,
    );
    notices.extend(finished_park_notices(caches.parked_dir.as_deref()));
    if let Some(messaging) = caches.messaging.as_mut() {
        messaging.tick(registry, cli_interpreter, &panes, wall_now);
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

pub fn transition_line(
    id: &str,
    tool: &str,
    prev: Status,
    new: Status,
    reason: Reason,
    grace_secs: u64,
    evidence: &Evidence,
) -> String {
    format!(
        "[cli-sessions] transition id={id} tool={tool} prev={prev:?} new={new:?} reason={reason:?} grace_s={grace_secs} evidence=dr:{:?} sr:{:?} vp:{:?} fresh:{:?} quiet:{:?} moved:{} prompt:{}",
        evidence.descriptor_runtime,
        evidence.screen_runtime,
        evidence.viewport,
        evidence.file_fresh,
        evidence.file_quiet_secs,
        evidence.screen_changed,
        evidence.at_prompt
    )
}

fn attention_notice(
    session: &SessionId,
    prev: Status,
    new: Status,
    tool: &Tool,
    label: Option<&str>,
    cwd: &str,
    summary: &str,
) -> Option<Notice> {
    notify::announces_attention(prev, new).then(|| {
        let name = label.map(str::to_string).unwrap_or_else(|| project_of(cwd));
        Notice::new(session.clone(), tool, name, summary)
    })
}

fn cached_screen(
    host: &dyn TerminalHost,
    cli_interpreter: &CliSessionInterpreter,
    pane: &Pane,
    cli_session: &CliSessionDescriptor,
    refresh_active: bool,
    now: u64,
    caches: &mut ReconcileCaches,
) -> Option<Arc<ScreenAnalysis>> {
    let id = pane.id.clone();
    let identity = ScreenIdentity::new(pane, cli_session);
    let replace = caches
        .screens
        .get(&id)
        .is_none_or(|entry| entry.identity != identity);
    if replace {
        caches.screens.insert(
            id.clone(),
            ScreenCache {
                identity,
                analysis: None,
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
    let reason = refresh_active
        .then_some("active")
        .or_else(|| entry.analysis.is_none().then_some("initial"))
        .or_else(|| {
            entry
                .analysis
                .as_ref()
                .is_some_and(|screen| screen.pane != *pane)
                .then_some("pane_changed")
        })
        .or_else(|| entry.subscription.is_none().then_some("unsubscribed"))
        .or_else(|| signaled.then_some("signal"))
        .or_else(|| {
            (now.saturating_sub(entry.last_read) >= SCREEN_FALLBACK_SECS).then_some("fallback")
        });
    let Some(reason) = reason else {
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=screen id={id} source=cache age_secs={} subscription=active",
            now.saturating_sub(entry.last_read)
        );
        return entry.analysis.clone();
    };
    #[cfg(debug_assertions)]
    let started = std::time::Instant::now();
    let fresh = pane
        .binding()
        .ok()
        .and_then(|binding| host.get_text(&binding));
    #[cfg(debug_assertions)]
    let elapsed_ms = started.elapsed().as_millis();
    #[cfg(not(debug_assertions))]
    let elapsed_ms = 0_u128;
    if let Some(text) = fresh {
        let mut analysis = ScreenAnalysis::refresh(
            entry.analysis.as_ref(),
            text,
            pane,
            &cli_session.tool,
            cli_interpreter,
        );
        let reused = entry
            .analysis
            .as_ref()
            .is_some_and(|previous| Arc::ptr_eq(previous, &analysis));
        let mut cacheable = true;
        if !reused
            && analysis.evidence.runtime == CliRuntimeState::NeedsInput
            && analysis.evidence.viewport == CliViewportState::Live
        {
            let current = pane
                .binding()
                .ok()
                .and_then(|binding| host.visual_screen_is_current(&binding, &analysis.text));
            if current != Some(true) {
                Arc::get_mut(&mut analysis)
                    .expect("a new screen analysis has one owner")
                    .evidence
                    .viewport = CliViewportState::Historical;
                cacheable = current.is_some();
            }
        }
        #[cfg(debug_assertions)]
        let analysis_reused = reused;
        #[cfg(not(debug_assertions))]
        let analysis_reused = false;
        entry.analysis = cacheable.then(|| analysis.clone());
        if !cacheable {
            entry.dirty.store(true, Ordering::Release);
        }
        entry.last_read = now;
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=screen id={id} source=read reason={reason} elapsed_ms={elapsed_ms} analysis_reused={analysis_reused} subscription={}",
            if entry.subscription.is_some() {
                "active"
            } else {
                "unsupported"
            }
        );
        return Some(analysis);
    }
    entry.dirty.store(true, Ordering::Release);
    qol_runtime::probe!(
        "CLI_SESSIONS_RECON",
        "phase=screen id={id} source=read reason={reason} outcome=unavailable elapsed_ms={elapsed_ms}"
    );
    None
}

fn prune_missing(registry: &Arc<Mutex<Registry>>, caches: &mut ReconcileCaches, panes: &[Pane]) {
    let live: HashSet<SessionId> = panes.iter().map(|pane| pane.id.clone()).collect();
    caches.screens.retain(|id, _| live.contains(id));
    let Ok(mut reg) = registry.lock() else { return };
    reg.prune(pid_alive);
    let stale: Vec<SessionId> = reg
        .sorted()
        .into_iter()
        .filter(|s| s.park.is_none() && !live.contains(&s.id))
        .map(|s| s.id)
        .collect();
    #[cfg(debug_assertions)]
    if !stale.is_empty() {
        qol_runtime::probe!("CLI_SESSIONS_RECON", "phase=prune removed={:?}", stale);
    }
    for id in stale {
        reg.remove(&id);
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

fn snapshot(registry: &Arc<Mutex<Registry>>, id: &SessionId) -> (Attention, Option<u64>) {
    let Ok(reg) = registry.lock() else {
        return (Attention::default(), None);
    };
    match reg.get(id) {
        Some(s) => (
            Attention {
                status: s.runtime_status.unwrap_or(match s.status {
                    Status::Coordinating | Status::AwaitingReview | Status::Parked => {
                        Status::Unknown
                    }
                    Status::Working
                    | Status::Service
                    | Status::YourTurn
                    | Status::NeedsYou
                    | Status::Unknown
                    | Status::Acknowledged => s.status,
                }),
                working_since: s.working_since,
                settled_since: s.settled_since,
            },
            s.screen_hash,
        ),
        None => (Attention::default(), None),
    }
}

pub struct ApplyInput<'a> {
    pub pane: &'a Pane,
    pub tool: Tool,
    pub label: Option<&'a str>,
    pub reduction: Reduction,
    pub evidence: &'a Evidence,
    pub new_hash: Option<u64>,
    pub branch: Option<String>,
    pub now: u64,
    pub wall_now: u64,
    pub bridged: bool,
    pub driving: Vec<SessionId>,
    pub parked: bool,
    /// A woken park session its runner closes when the turn ends; its report replaces the toast.
    pub closing: bool,
}

fn apply(reg: &mut Registry, input: ApplyInput) -> (Option<Notice>, Status) {
    let pane_id = &input.pane.id;
    let prev_status = match reg.get(pane_id) {
        Some(s) => s.status,
        None => Status::Unknown,
    };
    let previous_name = reg
        .get(pane_id)
        .filter(|s| s.root_pid == input.pane.root_pid)
        .and_then(|s| meaningful_name(s.name.as_deref()))
        .map(str::to_owned);
    let name = meaningful_name(input.label)
        .map(str::to_owned)
        .or_else(|| previous_name.clone());
    #[cfg(debug_assertions)]
    if meaningful_name(input.label).is_none() && previous_name.is_some() {
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=label id={} action=retain previous={:?} incoming={:?}",
            pane_id,
            previous_name,
            input.label
        );
    }
    let status = crate::session::status::park_status(
        crate::session::status::bridge_status(
            input.reduction.attention.status,
            input.bridged,
            !input.driving.is_empty(),
        ),
        input.parked,
    );
    if status != input.reduction.attention.status {
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=bridge id={} runtime={:?} display={:?} delegated={} agents={} parked={}",
            pane_id,
            input.reduction.attention.status,
            status,
            input.bridged,
            input.driving.len(),
            input.parked
        );
    }
    let key = input
        .pane
        .spawn_identity
        .as_ref()
        .map(|identity| identity.key.as_str().to_owned());
    let master = is_master_key(key.as_deref());
    let summary = summary_for(status, &input.tool);
    let quiet = (input.closing && status == Status::YourTurn) || master_ready(master, status);
    let notice = (!quiet)
        .then(|| {
            attention_notice(
                pane_id,
                prev_status,
                status,
                &input.tool,
                name.as_deref(),
                &input.pane.cwd,
                &summary,
            )
        })
        .flatten();

    if status != prev_status {
        let reason = input
            .reduction
            .transition
            .map(|transition| transition.reason)
            .unwrap_or(Reason::LiveWork);
        let grace = reg
            .get(pane_id)
            .and_then(|s| s.settled_since)
            .map(|start| input.now.saturating_sub(start))
            .unwrap_or(0);
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=transition {}",
            transition_line(
                &pane_id.to_string(),
                tool_id(&input.tool),
                prev_status,
                status,
                reason,
                grace,
                input.evidence,
            )
        );
    }

    if let Some(s) = reg.get_mut(pane_id) {
        if status != s.status {
            s.last_activity = input.wall_now;
        }
        s.status = status;
        s.runtime_status = Some(input.reduction.attention.status);
        s.tool = input.tool;
        s.summary = summary;
        s.root_pid = input.pane.root_pid;
        s.project = project_of(&input.pane.cwd);
        s.cwd = input.pane.cwd.clone();
        s.branch = input.branch;
        s.name = name;
        s.working_since = input.reduction.attention.working_since;
        s.settled_since = input.reduction.attention.settled_since;
        s.bridged = input.bridged;
        s.driving = input.driving;
        s.key = key;
        s.master = master;
        if let Some(h) = input.new_hash {
            s.screen_hash = Some(h);
        }
        return (notice, status);
    }
    reg.upsert(SessionState {
        id: pane_id.clone(),
        root_pid: input.pane.root_pid,
        project: project_of(&input.pane.cwd),
        name,
        cwd: input.pane.cwd.clone(),
        branch: input.branch,
        tool: input.tool,
        status,
        summary,
        last_activity: input.wall_now,
        screen_hash: input.new_hash,
        key,
        master,
        working_since: input.reduction.attention.working_since,
        settled_since: input.reduction.attention.settled_since,
        bridged: input.bridged,
        driving: input.driving,
        runtime_status: Some(input.reduction.attention.status),
        unread: 0,
        open_asks: 0,
        park: None,
    });
    (notice, status)
}

pub struct LivePark {
    pub id: SessionId,
    pub runner_pid: i32,
    pub record: ParkRecord,
}

pub fn live_parks(
    records: Vec<ParkRecord>,
    runner_alive: impl Fn(u32, Option<&str>) -> bool,
) -> Vec<LivePark> {
    records
        .into_iter()
        .filter_map(|record| {
            let pid = record.runner_pid?;
            if !runner_alive(pid, record.runner_identity.as_deref()) {
                return None;
            }
            let binding = record.session.parse::<SessionBinding>().ok()?;
            Some(LivePark {
                id: binding.session_id().clone(),
                runner_pid: i32::try_from(pid).ok()?,
                record,
            })
        })
        .collect()
}

fn live_parked_sessions(dir: Option<&std::path::Path>) -> Vec<LivePark> {
    if let Some(dir) = dir {
        for id in qol_terminal_sessions::park::fail_stale(dir, park_runner_alive) {
            log::warn!("[cli-sessions] park {id} lost its runner before delivering");
        }
    }
    let records = dir
        .map(qol_terminal_sessions::park::waiting)
        .unwrap_or_default();
    live_parks(records, park_runner_alive)
}

fn park_runner_alive(pid: u32, identity: Option<&str>) -> bool {
    match identity {
        Some(identity) => qol_process::process_identity_matches(pid, identity),
        None => qol_process::is_pid_alive(pid),
    }
}

fn closing_sessions(dir: Option<&std::path::Path>) -> Vec<String> {
    dir.map(qol_terminal_sessions::park::watching)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| {
            record
                .runner_pid
                .is_some_and(|pid| park_runner_alive(pid, record.runner_identity.as_deref()))
        })
        .filter_map(|record| record.resumed_session)
        .collect()
}

/// One toast per finished or unresumable park, marked first so a failed write never repeats it.
fn finished_park_notices(dir: Option<&std::path::Path>) -> Vec<Notice> {
    let Some(dir) = dir else {
        return Vec::new();
    };
    qol_terminal_sessions::park::unnotified(dir)
        .into_iter()
        .filter(
            |record| match qol_terminal_sessions::park::mark_notified(dir, record) {
                Ok(()) => true,
                Err(error) => {
                    log::warn!(
                        "[cli-sessions] finished park {} not announced: {error}",
                        record.id
                    );
                    false
                }
            },
        )
        .map(|record| Notice::for_park(&record))
        .collect()
}

fn show_detached_parks(
    registry: &Arc<Mutex<Registry>>,
    cli_interpreter: &CliSessionInterpreter,
    caches: &mut ReconcileCaches,
    panes: &[Pane],
    parked: &[LivePark],
    open_conversations: &HashSet<String>,
    wall_now: u64,
) {
    let closed = caches
        .parked_dir
        .as_deref()
        .map(qol_terminal_sessions::park::closed)
        .unwrap_or_default();
    let store = caches
        .parked_dir
        .clone()
        .map(crate::park::store::ParkStore::with_dir);
    let closed_tab = |record: &ParkRecord| {
        record
            .resumed_session
            .as_deref()
            .unwrap_or(&record.session)
            .parse::<SessionBinding>()
            .ok()
    };
    if let Some(store) = store.as_ref() {
        for record in closed.iter().filter(|record| {
            open_conversations.contains(&record.external_id)
                && !closed_tab(record)
                    .is_some_and(|tab| panes.iter().any(|pane| pane.id == *tab.session_id()))
        }) {
            if let Err(error) = store.mark_reopened(record) {
                log::warn!(
                    "[cli-sessions] park {} not marked reopened: {error:#}",
                    record.id
                );
            }
        }
    }
    let mut detached: Vec<(SessionId, i32, &ParkRecord, u64)> = Vec::new();
    let waiting = parked.iter().map(|park| {
        (
            park.id.clone(),
            park.runner_pid,
            &park.record,
            park.record.created_at,
        )
    });
    let closed = closed.iter().filter_map(|record| {
        let binding = closed_tab(record)?;
        Some((
            binding.session_id().clone(),
            binding.root_pid(),
            record,
            record.closed_at?,
        ))
    });
    for (id, root_pid, record, since) in waiting.chain(closed) {
        if panes.iter().any(|pane| pane.id == id)
            || open_conversations.contains(&record.external_id)
            || detached.iter().any(|(kept, ..)| *kept == id)
        {
            continue;
        }
        detached.push((id, root_pid, record, since));
    }
    let rows = detached
        .into_iter()
        .map(|(id, root_pid, record, since)| {
            let tool = CliToolId::new(record.tool.as_str())
                .ok()
                .and_then(|id| cli_interpreter.tool_for(&id))
                .unwrap_or_else(generic_tool);
            let key = store
                .as_ref()
                .and_then(|store| store.spawn_key(&record.id))
                .map(|key| key.as_str().to_owned());
            SessionState {
                id,
                root_pid,
                project: project_of(&record.cwd),
                name: meaningful_name(record.title.as_deref()).map(str::to_owned),
                cwd: record.cwd.clone(),
                branch: caches.branch.branch(&record.cwd, wall_now),
                tool,
                status: Status::Parked,
                summary: Status::Parked.definition().label.into(),
                last_activity: since,
                screen_hash: None,
                master: is_master_key(key.as_deref()),
                key,
                working_since: None,
                settled_since: None,
                bridged: false,
                driving: Vec::new(),
                runtime_status: None,
                unread: 0,
                open_asks: 0,
                park: Some(record.id.clone()),
            }
        })
        .collect::<Vec<_>>();
    let Ok(mut reg) = registry.lock() else { return };
    let gone: Vec<SessionId> = reg
        .sorted()
        .into_iter()
        .filter(|state| state.park.is_some() && !rows.iter().any(|row| row.id == state.id))
        .map(|state| state.id)
        .collect();
    for id in gone {
        reg.remove(&id);
    }
    for row in rows {
        if reg
            .get(&row.id)
            .is_some_and(|state| state.root_pid == row.root_pid && state.park == row.park)
        {
            continue;
        }
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=park id={} park={} root_pid={}",
            row.id,
            row.park.as_deref().unwrap_or_default(),
            row.root_pid
        );
        reg.upsert(row);
    }
}

fn live_bridge_sessions(
    reader: &mut LiveBridgeReader,
) -> qol_terminal_sessions::bridge::LiveBridges {
    qol_terminal_sessions::bridge::checkpoint_dir()
        .map(|dir| reader.read(&dir))
        .unwrap_or_default()
}

fn tool_id(tool: &Tool) -> &str {
    tool.id.as_str()
}

#[cfg(test)]
mod bridge_tests {
    use super::*;
    use crate::attention::Phase;
    use qol_terminal_sessions::cli::claude_tool;

    fn pane() -> Pane {
        Pane {
            id: crate::host::kitty_session_id(1),
            root_pid: 1,
            cwd: "/project".into(),
            title: "architect".into(),
            at_prompt: false,
            focused: false,
            reported_cmd: Some("claude".into()),
            foreground_basenames: vec!["claude".into()],
            foreground_pids: Vec::new(),
            capabilities: qol_terminal_sessions::SessionCapabilities::ALL,
            spawn_identity: None,
        }
    }

    fn frame(
        reg: &mut Registry,
        pane: &Pane,
        runtime: Status,
        bridged: bool,
        driving: bool,
    ) -> (Option<Notice>, Status) {
        apply(
            reg,
            ApplyInput {
                pane,
                tool: claude_tool(),
                label: Some("architect"),
                reduction: Reduction {
                    attention: Attention {
                        status: runtime,
                        ..Attention::default()
                    },
                    phase: Phase::Hold,
                    transition: None,
                },
                evidence: &Evidence::default(),
                new_hash: None,
                branch: None,
                now: 100,
                wall_now: 100,
                bridged,
                driving: driving
                    .then(|| crate::host::kitty_session_id(2))
                    .into_iter()
                    .collect(),
                parked: false,
                closing: false,
            },
        )
    }

    #[test]
    fn open_agent_loops_suppress_human_attention_until_the_loop_closes() {
        for (bridged, driving, expected) in [
            (false, true, Status::Coordinating),
            (true, false, Status::AwaitingReview),
            (true, true, Status::Coordinating),
        ] {
            let pane = pane();
            let reg = Arc::new(Mutex::new(Registry::default()));
            for _ in 0..3 {
                let (notice, status) = frame(
                    &mut reg.lock().unwrap(),
                    &pane,
                    Status::YourTurn,
                    bridged,
                    driving,
                );
                assert_eq!(status, expected);
                assert!(notice.is_none());
                assert!(!status.is_attention());
                assert_eq!(snapshot(&reg, &pane.id).0.status, Status::YourTurn);
            }
            let (notice, status) = frame(
                &mut reg.lock().unwrap(),
                &pane,
                Status::YourTurn,
                false,
                false,
            );
            assert_eq!(status, Status::YourTurn);
            assert!(notice.is_some());
        }
    }

    #[test]
    fn human_approval_still_interrupts_an_agent_loop() {
        let pane = pane();
        let mut reg = Registry::default();
        frame(&mut reg, &pane, Status::Working, false, true);
        let (notice, status) = frame(&mut reg, &pane, Status::NeedsYou, false, true);
        assert_eq!(status, Status::NeedsYou);
        assert!(notice.is_some());
    }

    fn switchboard_pane() -> Pane {
        let mut pane = pane();
        pane.spawn_identity = Some(qol_terminal_sessions::SpawnIdentity {
            key: qol_terminal_sessions::SpawnKey::new(qol_sessions_engine::SWITCHBOARD_KEY)
                .unwrap(),
            tool: CliToolId::new("claude").unwrap(),
            surface: qol_terminal_sessions::SpawnSurface::OsWindow,
        });
        pane
    }

    #[test]
    fn the_switchboard_resting_at_its_prompt_never_raises_a_toast() {
        let pane = switchboard_pane();
        let mut reg = Registry::default();
        for runtime in [
            Status::Working,
            Status::YourTurn,
            Status::Working,
            Status::Unknown,
            Status::YourTurn,
        ] {
            let (notice, status) = frame(&mut reg, &pane, runtime, false, false);
            assert_eq!(status, runtime);
            assert!(notice.is_none(), "{runtime:?}");
        }
        let row = reg.get(&pane.id).unwrap();
        assert!(row.master);
        assert_eq!(
            row.key.as_deref(),
            Some(qol_sessions_engine::SWITCHBOARD_KEY)
        );
        assert!(row.is_master_ready());
    }

    #[test]
    fn the_switchboard_still_alerts_when_it_needs_the_user() {
        let pane = switchboard_pane();
        let mut reg = Registry::default();
        frame(&mut reg, &pane, Status::Working, false, false);
        let (notice, status) = frame(&mut reg, &pane, Status::NeedsYou, false, false);
        assert_eq!(status, Status::NeedsYou);
        assert!(notice.is_some());
    }

    #[test]
    fn a_lane_keeps_its_spawn_key_and_is_never_the_master() {
        let mut pane = switchboard_pane();
        if let Some(identity) = pane.spawn_identity.as_mut() {
            identity.key = qol_terminal_sessions::SpawnKey::new("lane-7").unwrap();
        }
        let mut reg = Registry::default();
        let (notice, _) = frame(&mut reg, &pane, Status::YourTurn, false, false);
        assert!(notice.is_some());
        let row = reg.get(&pane.id).unwrap();
        assert!(!row.master);
        assert_eq!(row.key.as_deref(), Some("lane-7"));
        frame(&mut reg, &self::pane(), Status::YourTurn, false, false);
        assert_eq!(reg.get(&pane.id).unwrap().key, None);
    }
}

#[cfg(test)]
mod attention_snapshot_tests {
    use super::*;

    #[test]
    fn statuses_map_to_attention_and_the_master_reads_ready_when_settled() {
        let cases = [
            (
                Status::NeedsYou,
                AttentionState::NeedsYou,
                AttentionState::NeedsYou,
            ),
            (
                Status::YourTurn,
                AttentionState::YourTurn,
                AttentionState::Ready,
            ),
            (
                Status::Acknowledged,
                AttentionState::YourTurn,
                AttentionState::Ready,
            ),
            (Status::Unknown, AttentionState::Idle, AttentionState::Ready),
            (
                Status::Working,
                AttentionState::Working,
                AttentionState::Working,
            ),
            (
                Status::Service,
                AttentionState::Working,
                AttentionState::Working,
            ),
            (
                Status::Coordinating,
                AttentionState::Working,
                AttentionState::Working,
            ),
            (
                Status::AwaitingReview,
                AttentionState::Working,
                AttentionState::Working,
            ),
            (
                Status::Parked,
                AttentionState::Parked,
                AttentionState::Parked,
            ),
        ];
        for (status, session, master) in cases {
            assert_eq!(attention_state(status, false), session, "{status:?}");
            assert_eq!(attention_state(status, true), master, "{status:?} master");
        }
    }

    #[test]
    fn the_snapshot_is_rewritten_on_change_and_on_the_heartbeat() {
        let row = |state| AttentionRow {
            key: Some("lane".into()),
            title: "lane".into(),
            state,
            master: false,
        };
        let written = (vec![row(AttentionState::Working)], 100);
        let cases = [
            (None, AttentionState::Working, 100, true),
            (Some(&written), AttentionState::Working, 100, false),
            (Some(&written), AttentionState::Working, 109, false),
            (Some(&written), AttentionState::Working, 110, true),
            (Some(&written), AttentionState::YourTurn, 101, true),
        ];
        for (last, state, now, expected) in cases {
            assert_eq!(
                attention_due(last, &[row(state)], now),
                expected,
                "{state:?} at {now}"
            );
        }
    }
}
