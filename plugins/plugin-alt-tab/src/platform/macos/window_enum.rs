use super::process::{
    cached_process_identity, is_regular_app, known_window_ids_by_identity, ProcessIdentity,
};
use super::{ax, AxWindowMeta, CgWindow};
use super::ax::AxCache;
use crate::platform::WindowInfo;
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
            snapshot,
            accepted: HashMap::new(),
            seen: HashSet::new(),
            identity_cache: HashMap::new(),
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
        let Some(identity) = self.identity_for_pid(pid) else {
            return;
        };
        self.accepted.entry(identity).or_default().insert(window_id);
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

fn allowed_minimized_count(
    on_screen_count: usize,
    identity: Option<ProcessIdentity>,
    snapshot: &HashMap<ProcessIdentity, HashSet<u32>>,
    id_map: &HashMap<u32, AxWindowMeta>,
    all_meta: &[AxWindowMeta],
    accepted_count: usize,
) -> usize {
    fn ax_minimized_count(id_map: &HashMap<u32, AxWindowMeta>, all_meta: &[AxWindowMeta]) -> usize {
        if !id_map.is_empty() {
            return id_map.values().filter(|w| w.is_minimized).count();
        }
        all_meta.iter().filter(|w| w.is_minimized).count()
    }

    let has_ax = !id_map.is_empty() || !all_meta.is_empty();

    if has_ax && on_screen_count != 0 {
        return ax_minimized_count(id_map, all_meta);
    }

    let snapshot_count = identity
        .filter(|_| on_screen_count == 0)
        .and_then(|id| snapshot.get(&id))
        .map(|ids| ids.len())
        .filter(|c| *c > 0);
    if let Some(count) = snapshot_count {
        return count;
    }

    if has_ax {
        return ax_minimized_count(id_map, all_meta);
    }

    if accepted_count > on_screen_count {
        return accepted_count - on_screen_count;
    }

    if accepted_count > 0 {
        return 0;
    }

    usize::MAX
}

pub(super) fn collect_on_screen_windows(
    parsed: Vec<CgWindow>,
    state: &mut WindowEnumeration,
    tracker: &mut KnownWindowTracker,
    ax_cache: &mut AxCache,
) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/enum] CG on-screen: {} windows", parsed.len());

    // Count ALL regular-app on-screen windows per PID (including tiny ones)
    // so that on_screen_count matches AX accepted_count for budget accuracy.
    // Tiny windows inflating accepted_count but not on_screen_count causes
    // phantom minimized budget during rapid focus transitions.
    let mut regular_cache: HashMap<i32, bool> = HashMap::new();
    for w in &parsed {
        state.on_screen_ids.insert(w.id);
        if *regular_cache.entry(w.pid).or_insert_with(|| is_regular_app(w.pid)) {
            state.register_on_screen_pid(w.pid);
        }
    }

    const MIN_WINDOW_DIM: f32 = 100.0;
    let parsed: Vec<CgWindow> = parsed
        .into_iter()
        .filter(|w| {
            if w.w < MIN_WINDOW_DIM || w.h < MIN_WINDOW_DIM {
                #[cfg(debug_assertions)]
                eprintln!("[alt-tab/enum] FILTERED (too small {}x{}): wid={} app={:?} title={:?}", w.w, w.h, w.id, w.app_name, w.title);
                return false;
            }
            let is_regular = regular_cache[&w.pid];
            #[cfg(debug_assertions)]
            if !is_regular {
                eprintln!("[alt-tab/enum] FILTERED (not regular app): wid={} pid={} app={:?}", w.id, w.pid, w.app_name);
            }
            is_regular
        })
        .collect();

    #[cfg(debug_assertions)]
    for w in &parsed {
        eprintln!("[alt-tab/enum] pre-dedup: wid={} pid={} app={:?} title={:?}", w.id, w.pid, w.app_name, w.title);
    }

    let deduped = ax::dedup_by_ax(parsed, ax_cache);
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/enum] post-dedup: {} windows", deduped.len());
    for window in deduped {
        tracker.remember_window(window.pid, window.id);
        state.push_on_screen(window);
    }
}

fn detect_other_space_pids(
    state: &WindowEnumeration,
    ax_cache: &mut AxCache,
) -> HashSet<i32> {
    let mut result = HashSet::new();
    for &pid in &state.on_screen_pids {
        let entry = ax_cache.entry(pid).or_insert_with(|| ax::ax_windows(pid));
        let Some((id_map, _, accepted)) = entry else { continue };
        // Without _AXWindowID (id_map empty), accepted count includes transient
        // windows that inflate the count — can't reliably distinguish other-space
        // windows from transient overlays. Skip to force the stricter
        // ax_is_window_minimized path instead of the permissive ax_is_window_real.
        if id_map.is_empty() {
            continue;
        }
        if *accepted > state.on_screen_count(pid) {
            result.insert(pid);
        }
    }
    result
}

fn passes_ax_filter(
    window: &CgWindow,
    is_on_screen_pid: bool,
    is_other_space_pid: bool,
    ax_cache: &AxCache,
) -> bool {
    if !is_on_screen_pid {
        return true;
    }
    // Use cached ax_windows() data instead of per-window ax_find_window() calls.
    // detect_other_space_pids already populated ax_cache for all on-screen PIDs.
    if let Some(Some((id_map, all_meta, _))) = ax_cache.get(&window.pid) {
        // Prefer id_map when the app supports _AXWindowID
        if !id_map.is_empty() {
            if is_other_space_pid {
                return id_map.contains_key(&window.id);
            }
            return id_map.get(&window.id).map_or(false, |m| m.is_minimized);
        }
        // Fall back to title matching in all_meta (avoids per-window ax_find_window)
        if !all_meta.is_empty() {
            let title_match = all_meta.iter().find(|m| m.title == window.title);
            if is_other_space_pid {
                return title_match.is_some();
            }
            if let Some(meta) = title_match {
                return meta.is_minimized;
            }
            // No title match — single-window fallback (matches ax_find_window behavior)
            if all_meta.len() == 1 {
                return all_meta[0].is_minimized;
            }
            return false;
        }
    }
    // No cache entry — per-window fallback
    if is_other_space_pid {
        return ax::ax_is_window_real(window.pid, window.id, &window.title);
    }
    ax::ax_is_window_minimized(window.pid, window.id, &window.title)
}

struct ResolvedWindow {
    title: String,
    allowed_count: usize,
}

fn resolve_minimized_budget(
    window: &CgWindow,
    state: &WindowEnumeration,
    tracker: &mut KnownWindowTracker,
    ax_cache: &mut AxCache,
) -> Option<ResolvedWindow> {
    let on_screen_count = state.on_screen_count(window.pid);
    let identity = tracker.identity_for_pid(window.pid);
    let known_ids = if on_screen_count == 0 {
        identity.and_then(|id| tracker.snapshot.get(&id))
    } else {
        None
    };
    let known_budget = known_ids
        .map(|ids| ids.len())
        .filter(|count| *count > 0);

    let ax_windows = ax_cache
        .entry(window.pid)
        .or_insert_with(|| ax::ax_windows(window.pid));
    let mut title = window.title.clone();
    let mut allowed_count = known_budget.unwrap_or(usize::MAX);

    let ax_meta = ax_windows.as_ref().and_then(|(id_map, all_meta, _)| {
        id_map.get(&window.id).or_else(|| {
            all_meta.iter().find(|m| m.title == window.title)
        })
    });
    let ax_has_window = ax_meta.is_some();
    let ax_is_minimized = ax_meta.map_or(false, |m| m.is_minimized);

    if let Some(meta) = ax_meta.filter(|m| !m.title.is_empty()) {
        title = meta.title.clone();
    }
    if let Some((id_map, all_meta, accepted_count)) = ax_windows.as_ref().filter(|_| known_budget.is_none()) {
        allowed_count =
            allowed_minimized_count(on_screen_count, identity, &tracker.snapshot, id_map, all_meta, *accepted_count);
    }

    if known_budget.is_none() && ax_has_window && !ax_is_minimized {
        return None;
    }
    let accepted_count = ax_windows.as_ref().map_or(0, |(_, _, c)| *c);
    if known_ids.is_some_and(|ids| !ids.is_empty() && !ax_has_window && accepted_count == 0 && !ids.contains(&window.id)) {
        return None;
    }

    Some(ResolvedWindow { title, allowed_count })
}

pub(super) fn collect_minimized_windows(
    off_screen: Vec<CgWindow>,
    state: &mut WindowEnumeration,
    tracker: &mut KnownWindowTracker,
    ax_cache: &mut AxCache,
) {
    let mut regular_app_cache: HashMap<i32, bool> = HashMap::new();
    let mut minimized_count_by_pid: HashMap<i32, usize> = HashMap::new();
    let mut seen_ids = HashSet::new();

    let other_space_pids = detect_other_space_pids(state, ax_cache);

    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/enum] CG off-screen candidates: {}", off_screen.len());

    for window in off_screen {
        if state.on_screen_ids.contains(&window.id) {
            continue;
        }
        if !seen_ids.insert(window.id) {
            continue;
        }
        if !window.has_title || window.w < 1.0 || window.h < 1.0 {
            continue;
        }
        let is_regular = *regular_app_cache
            .entry(window.pid)
            .or_insert_with(|| is_regular_app(window.pid));
        if !is_regular {
            continue;
        }

        let is_on_screen_pid = state.on_screen_pids.contains(&window.pid);
        let is_other_space_pid = other_space_pids.contains(&window.pid);
        if !passes_ax_filter(&window, is_on_screen_pid, is_other_space_pid, ax_cache) {
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/enum] MINIMIZED skip (AX filter): wid={} app={:?}", window.id, window.app_name);
            continue;
        }

        let Some(resolved) = resolve_minimized_budget(&window, state, tracker, ax_cache) else {
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/enum] MINIMIZED skip (budget rejected): wid={} app={:?}", window.id, window.app_name);
            continue;
        };
        if resolved.allowed_count == 0 {
            continue;
        }
        let current_count = minimized_count_by_pid.entry(window.pid).or_insert(0);
        if *current_count >= resolved.allowed_count {
            continue;
        }
        *current_count += 1;

        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/enum] MINIMIZED accepted: wid={} app={:?} title={:?} sharing={}", window.id, window.app_name, resolved.title, window.sharing_state);
        tracker.remember_window(window.pid, window.id);
        state.push_minimized(&window, resolved.title);
    }

    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/enum] total windows after minimized: {}", state.windows.len());
}
