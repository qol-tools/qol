use super::process::{
    cached_process_identity, is_regular_app, known_window_ids_by_identity, ProcessIdentity,
};
use super::{ax, CgWindow};
use super::ax::{AxCache, AxWindowMeta};
use crate::discovery::WindowInfo;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct WindowEnumeration {
    pub windows: Vec<WindowInfo>,
    pub on_screen_ids: HashSet<u32>,
    pub on_screen_pids: HashSet<i32>,
    on_screen_count_by_pid: HashMap<i32, usize>,
}

impl WindowEnumeration {
    pub fn on_screen_count(&self, pid: i32) -> usize {
        self.on_screen_count_by_pid.get(&pid).copied().unwrap_or(0)
    }

    fn register_on_screen_pid(&mut self, pid: i32) {
        self.on_screen_pids.insert(pid);
        *self.on_screen_count_by_pid.entry(pid).or_insert(0) += 1;
    }

    pub fn push_on_screen(&mut self, window: CgWindow) {
        self.windows.push(window.into_window_info(false));
    }

    pub fn push_minimized(&mut self, window: &CgWindow, title: String) {
        self.windows.push(window.to_window_info(true, title));
    }
}

pub(super) struct KnownWindowTracker {
    pub snapshot: HashMap<ProcessIdentity, HashSet<u32>>,
    pub accepted: HashMap<ProcessIdentity, HashSet<u32>>,
    seen: HashSet<ProcessIdentity>,
    pub identity_cache: HashMap<i32, Option<ProcessIdentity>>,
}

impl KnownWindowTracker {
    pub fn new() -> Self {
        let snapshot = known_window_ids_by_identity()
            .lock()
            .ok()
            .map(|cache| cache.clone())
            .unwrap_or_default();
        Self {
            snapshot, accepted: HashMap::new(),
            seen: HashSet::new(), identity_cache: HashMap::new(),
        }
    }

    pub fn identity_for_pid(&mut self, pid: i32) -> Option<ProcessIdentity> {
        let identity = cached_process_identity(pid, &mut self.identity_cache);
        if let Some(identity) = identity {
            self.seen.insert(identity);
        }
        identity
    }

    pub fn remember_window(&mut self, pid: i32, window_id: u32) {
        let Some(identity) = self.identity_for_pid(pid) else { return };
        self.accepted.entry(identity).or_default().insert(window_id);
    }

    pub fn cached_identity(&self, pid: i32) -> Option<ProcessIdentity> {
        self.identity_cache.get(&pid).copied().flatten()
    }

    pub fn persist(self) {
        if let Ok(mut known_cache) = known_window_ids_by_identity().lock() {
            known_cache.retain(|identity, _| self.seen.contains(identity));
            for (identity, ids) in self.accepted {
                known_cache.insert(identity, ids);
            }
        }
    }
}

const MIN_WINDOW_DIM: f32 = 100.0;

pub(super) fn collect_on_screen_windows(
    parsed: Vec<CgWindow>,
    state: &mut WindowEnumeration,
    tracker: &mut KnownWindowTracker,
    ax_cache: &mut AxCache,
) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/enum] CG on-screen: {} windows", parsed.len());

    let regular_cache = register_on_screen(&parsed, state);
    let filtered = filter_visible(parsed, &regular_cache);
    #[cfg(debug_assertions)]
    for w in &filtered {
        eprintln!("[alt-tab/enum] pre-dedup: wid={} pid={} app={:?} title={:?}", w.id, w.pid, w.app_name, w.title);
    }
    let deduped = ax::dedup_by_ax(filtered, ax_cache);
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/enum] post-dedup: {} windows", deduped.len());
    for window in deduped {
        tracker.remember_window(window.pid, window.id);
        state.push_on_screen(window);
    }
}

fn register_on_screen(parsed: &[CgWindow], state: &mut WindowEnumeration) -> HashMap<i32, bool> {
    let mut regular_cache: HashMap<i32, bool> = HashMap::new();
    for w in parsed {
        state.on_screen_ids.insert(w.id);
        if *regular_cache.entry(w.pid).or_insert_with(|| is_regular_app(w.pid)) {
            state.register_on_screen_pid(w.pid);
        }
    }
    regular_cache
}

fn filter_visible(parsed: Vec<CgWindow>, regular_cache: &HashMap<i32, bool>) -> Vec<CgWindow> {
    parsed.into_iter().filter(|w| {
        if w.w < MIN_WINDOW_DIM || w.h < MIN_WINDOW_DIM {
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/enum] FILTERED (too small {}x{}): wid={} app={:?}", w.w, w.h, w.id, w.app_name);
            return false;
        }
        let is_regular = regular_cache[&w.pid];
        #[cfg(debug_assertions)]
        if !is_regular {
            eprintln!("[alt-tab/enum] FILTERED (not regular app): wid={} pid={} app={:?}", w.id, w.pid, w.app_name);
        }
        is_regular
    }).collect()
}

struct AxData<'a> {
    id_map: &'a HashMap<u32, AxWindowMeta>,
    all_meta: &'a [AxWindowMeta],
    accepted: usize,
}

impl AxData<'_> {
    fn has_data(&self) -> bool { !self.id_map.is_empty() || !self.all_meta.is_empty() }

    fn minimized_count(&self) -> usize {
        if !self.id_map.is_empty() {
            return self.id_map.values().filter(|w| w.is_minimized).count();
        }
        self.all_meta.iter().filter(|w| w.is_minimized).count()
    }
}

fn allowed_minimized_count(
    on_screen_count: usize, ax: &AxData,
    identity: Option<ProcessIdentity>, snapshot: &HashMap<ProcessIdentity, HashSet<u32>>,
) -> usize {
    if ax.has_data() && on_screen_count != 0 { return ax.minimized_count(); }
    let snap_count = identity
        .filter(|_| on_screen_count == 0)
        .and_then(|id| snapshot.get(&id))
        .map(|ids| ids.len())
        .filter(|c| *c > 0);
    if let Some(count) = snap_count { return count; }
    if ax.has_data() { return ax.minimized_count(); }
    if ax.accepted > on_screen_count { return ax.accepted - on_screen_count; }
    if ax.accepted > 0 { return 0; }
    usize::MAX
}

fn detect_other_space_pids(state: &WindowEnumeration, ax_cache: &mut AxCache) -> HashSet<i32> {
    let mut result = HashSet::new();
    for &pid in &state.on_screen_pids {
        let entry = ax_cache.entry(pid).or_insert_with(|| ax::ax_windows(pid));
        let Some((id_map, _, accepted)) = entry else { continue };
        // Without _AXWindowID (id_map empty), accepted count includes transient
        // windows — can't reliably distinguish other-space from transient overlays.
        if id_map.is_empty() { continue; }
        if *accepted > state.on_screen_count(pid) {
            result.insert(pid);
        }
    }
    result
}

fn passes_ax_filter(
    window: &CgWindow,
    state: &WindowEnumeration,
    other_space_pids: &HashSet<i32>,
    ax_cache: &AxCache,
) -> bool {
    let is_on_screen_pid = state.on_screen_pids.contains(&window.pid);
    let is_other_space_pid = other_space_pids.contains(&window.pid);
    if !is_on_screen_pid { return true; }
    let Some(Some((id_map, all_meta, _))) = ax_cache.get(&window.pid) else {
        return ax_fallback(window, is_other_space_pid);
    };
    if !id_map.is_empty() {
        if is_other_space_pid { return id_map.contains_key(&window.id); }
        return id_map.get(&window.id).map_or(false, |m| m.is_minimized);
    }
    if !all_meta.is_empty() {
        return ax_title_match(window, all_meta, is_other_space_pid);
    }
    ax_fallback(window, is_other_space_pid)
}

fn ax_title_match(window: &CgWindow, all_meta: &[AxWindowMeta], is_other_space: bool) -> bool {
    let title_match = all_meta.iter().find(|m| m.title == window.title);
    if is_other_space { return title_match.is_some(); }
    if let Some(meta) = title_match { return meta.is_minimized; }
    if all_meta.len() == 1 { return all_meta[0].is_minimized; }
    false
}

fn ax_fallback(window: &CgWindow, is_other_space: bool) -> bool {
    if is_other_space {
        return ax::ax_is_window_real(window.pid, window.id, &window.title);
    }
    ax::ax_is_window_minimized(window.pid, window.id, &window.title)
}

struct ResolvedWindow {
    title: String,
    allowed_count: usize,
}

struct BudgetContext {
    known_ids: Option<HashSet<u32>>,
    ax_meta: Option<AxWindowMeta>,
    ax_accepted: usize,
    allowed_count: usize,
}

fn build_budget_context(
    window: &CgWindow, state: &WindowEnumeration,
    tracker: &mut KnownWindowTracker, ax_cache: &mut AxCache,
) -> BudgetContext {
    tracker.identity_for_pid(window.pid);
    let ax = ax_cache.entry(window.pid).or_insert_with(|| ax::ax_windows(window.pid));
    let ax_meta = ax.as_ref().and_then(|(id_map, all_meta, _)| {
        id_map.get(&window.id).or_else(|| all_meta.iter().find(|m| m.title == window.title))
    }).cloned();
    let ax_accepted = ax.as_ref().map_or(0, |(_, _, c)| *c);
    let (known_ids, allowed_count) = compute_allowed_count(window, state, tracker, ax_cache);
    BudgetContext { known_ids, ax_meta, ax_accepted, allowed_count }
}

fn compute_allowed_count(
    window: &CgWindow, state: &WindowEnumeration,
    tracker: &KnownWindowTracker, ax_cache: &AxCache,
) -> (Option<HashSet<u32>>, usize) {
    let on_screen_count = state.on_screen_count(window.pid);
    let known_ids = tracker.cached_identity(window.pid)
        .filter(|_| on_screen_count == 0)
        .and_then(|id| tracker.snapshot.get(&id))
        .cloned();
    let known_budget = known_ids.as_ref().map(|ids| ids.len()).filter(|c| *c > 0);
    if let Some(b) = known_budget { return (known_ids, b); }
    let count = match ax_cache.get(&window.pid) {
        Some(Some((id_map, all_meta, accepted))) => {
            let ax = AxData { id_map, all_meta, accepted: *accepted };
            allowed_minimized_count(on_screen_count, &ax, tracker.cached_identity(window.pid), &tracker.snapshot)
        }
        _ => usize::MAX,
    };
    (known_ids, count)
}

fn resolve_minimized_budget(ctx: &BudgetContext, window: &CgWindow) -> Option<ResolvedWindow> {
    let ax_has = ctx.ax_meta.is_some();
    let ax_min = ctx.ax_meta.as_ref().map_or(false, |m| m.is_minimized);
    if ctx.known_ids.is_none() && ax_has && !ax_min { return None; }
    if let Some(ids) = &ctx.known_ids {
        if !ids.is_empty() && !ax_has && ctx.ax_accepted == 0 && !ids.contains(&window.id) {
            return None;
        }
    }
    let title = ctx.ax_meta.as_ref()
        .filter(|m| !m.title.is_empty())
        .map(|m| m.title.clone())
        .unwrap_or_else(|| window.title.clone());
    Some(ResolvedWindow { title, allowed_count: ctx.allowed_count })
}

pub(super) fn collect_minimized_windows(
    off_screen: Vec<CgWindow>,
    state: &mut WindowEnumeration,
    tracker: &mut KnownWindowTracker,
    ax_cache: &mut AxCache,
) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/enum] CG off-screen candidates: {}", off_screen.len());

    let other_space_pids = detect_other_space_pids(state, ax_cache);
    let candidates = filter_minimized_candidates(off_screen, state);
    let mut budget_counts: HashMap<i32, usize> = HashMap::new();

    for window in candidates {
        if !passes_ax_filter(&window, state, &other_space_pids, ax_cache) {
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/enum] MINIMIZED skip (AX filter): wid={} app={:?}", window.id, window.app_name);
            continue;
        }
        let Some(resolved) = try_accept_minimized(&window, state, tracker, ax_cache, &mut budget_counts) else {
            continue;
        };
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/enum] MINIMIZED accepted: wid={} app={:?} title={:?}", window.id, window.app_name, resolved.title);
        tracker.remember_window(window.pid, window.id);
        state.push_minimized(&window, resolved.title);
    }

    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/enum] total windows after minimized: {}", state.windows.len());
}

fn filter_minimized_candidates(off_screen: Vec<CgWindow>, state: &WindowEnumeration) -> Vec<CgWindow> {
    let mut seen_ids = HashSet::new();
    let mut regular_cache: HashMap<i32, bool> = HashMap::new();
    off_screen.into_iter().filter(|w| {
        if state.on_screen_ids.contains(&w.id) { return false; }
        if !seen_ids.insert(w.id) { return false; }
        if !w.has_title || w.w < 1.0 || w.h < 1.0 { return false; }
        *regular_cache.entry(w.pid).or_insert_with(|| is_regular_app(w.pid))
    }).collect()
}

fn try_accept_minimized(
    window: &CgWindow,
    state: &WindowEnumeration,
    tracker: &mut KnownWindowTracker,
    ax_cache: &mut AxCache,
    budget_counts: &mut HashMap<i32, usize>,
) -> Option<ResolvedWindow> {
    let ctx = build_budget_context(window, state, tracker, ax_cache);
    let resolved = resolve_minimized_budget(&ctx, window)?;
    if resolved.allowed_count == 0 { return None; }
    let count = budget_counts.entry(window.pid).or_insert(0);
    if *count >= resolved.allowed_count { return None; }
    *count += 1;
    Some(resolved)
}
