use super::ax::{AxCache, AxWindowMeta};
use super::process::{
    cached_process_identity, is_app_hidden, is_regular_app, known_window_ids_by_identity,
    ProcessIdentity,
};
use super::{ax, CgWindow};
use crate::discovery::WindowInfo;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
struct StableWindowKey {
    identity: ProcessIdentity,
    window_id: u32,
}

struct StableWindow {
    key: Option<StableWindowKey>,
    window: CgWindow,
}

static STABLE_WINDOW_ORDER: OnceLock<Mutex<Vec<StableWindowKey>>> = OnceLock::new();

fn stable_window_order() -> &'static Mutex<Vec<StableWindowKey>> {
    STABLE_WINDOW_ORDER.get_or_init(|| Mutex::new(Vec::new()))
}

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

    pub fn push_other_space(&mut self, window: &CgWindow, title: String) {
        self.windows.push(window.to_window_info(false, title));
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
        eprintln!(
            "[alt-tab/enum] pre-dedup: wid={} pid={} app={:?} title={:?}",
            w.id, w.pid, w.app_name, w.title
        );
    }
    let deduped = ax::dedup_by_ax(filtered, ax_cache);
    let deduped = stabilize_on_screen_order(deduped, tracker);
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/enum] post-dedup: {} windows", deduped.len());
    for window in deduped {
        tracker.remember_window(window.pid, window.id);
        state.push_on_screen(window);
    }
}

fn stabilize_on_screen_order(
    windows: Vec<CgWindow>,
    tracker: &mut KnownWindowTracker,
) -> Vec<CgWindow> {
    if windows.len() <= 1 {
        persist_stable_window_order(current_stable_keys(&windows, tracker));
        return windows;
    }

    let stable_windows = windows
        .into_iter()
        .map(|window| StableWindow {
            key: stable_window_key(&window, tracker),
            window,
        })
        .collect::<Vec<_>>();
    let current = stable_windows
        .iter()
        .filter_map(|window| window.key)
        .collect::<Vec<_>>();
    if current.is_empty() {
        persist_stable_window_order(Vec::new());
        return stable_windows
            .into_iter()
            .map(|window| window.window)
            .collect();
    }
    let previous = read_stable_window_order();
    let focused = stable_windows.first().and_then(|window| window.key);
    if use_current_window_order(focused, &previous) {
        persist_stable_window_order(current);
        return stable_windows
            .into_iter()
            .map(|window| window.window)
            .collect();
    }
    let ordered_keys = merge_stable_keys(focused, &previous, &current);
    let ordered = apply_stable_window_order(stable_windows, &ordered_keys);
    persist_stable_window_order(ordered_keys);
    ordered
}

fn stable_window_key(
    window: &CgWindow,
    tracker: &mut KnownWindowTracker,
) -> Option<StableWindowKey> {
    let identity = tracker.identity_for_pid(window.pid)?;
    Some(StableWindowKey {
        identity,
        window_id: window.id,
    })
}

fn current_stable_keys(
    windows: &[CgWindow],
    tracker: &mut KnownWindowTracker,
) -> Vec<StableWindowKey> {
    windows
        .iter()
        .filter_map(|window| stable_window_key(window, tracker))
        .collect()
}

fn read_stable_window_order() -> Vec<StableWindowKey> {
    stable_window_order()
        .lock()
        .ok()
        .map(|order| order.clone())
        .unwrap_or_default()
}

fn use_current_window_order(
    focused: Option<StableWindowKey>,
    previous: &[StableWindowKey],
) -> bool {
    focused.is_none() || previous.is_empty()
}

fn merge_stable_keys(
    focused: Option<StableWindowKey>,
    previous: &[StableWindowKey],
    current: &[StableWindowKey],
) -> Vec<StableWindowKey> {
    let current_set = current.iter().copied().collect::<HashSet<_>>();
    let mut result = Vec::with_capacity(current.len());
    let mut seen = HashSet::new();

    if let Some(focused) = focused.filter(|key| current_set.contains(key)) {
        push_unique_key(&mut result, &mut seen, focused);
    }
    for &key in previous {
        if !current_set.contains(&key) {
            continue;
        }
        push_unique_key(&mut result, &mut seen, key);
    }
    for &key in current {
        push_unique_key(&mut result, &mut seen, key);
    }
    result
}

fn push_unique_key(
    result: &mut Vec<StableWindowKey>,
    seen: &mut HashSet<StableWindowKey>,
    key: StableWindowKey,
) {
    if !seen.insert(key) {
        return;
    }
    result.push(key);
}

fn apply_stable_window_order(
    stable_windows: Vec<StableWindow>,
    ordered_keys: &[StableWindowKey],
) -> Vec<CgWindow> {
    let mut stable_windows = stable_windows.into_iter();
    let Some(focused) = stable_windows.next() else {
        return Vec::new();
    };

    let mut by_key = HashMap::new();
    let mut remaining_keys = Vec::new();
    let mut unknown = Vec::new();

    for stable in stable_windows {
        let Some(key) = stable.key else {
            unknown.push(stable.window);
            continue;
        };
        remaining_keys.push(key);
        by_key.insert(key, stable.window);
    }

    let mut result = Vec::with_capacity(ordered_keys.len() + unknown.len() + 1);
    let mut seen = HashSet::new();
    if let Some(key) = focused.key {
        seen.insert(key);
    }
    result.push(focused.window);

    for &key in ordered_keys {
        if seen.contains(&key) {
            continue;
        }
        let Some(window) = by_key.remove(&key) else {
            continue;
        };
        seen.insert(key);
        result.push(window);
    }

    for key in remaining_keys {
        if seen.contains(&key) {
            continue;
        }
        let Some(window) = by_key.remove(&key) else {
            continue;
        };
        seen.insert(key);
        result.push(window);
    }

    result.extend(unknown);
    result
}

fn persist_stable_window_order(order: Vec<StableWindowKey>) {
    let Ok(mut cached) = stable_window_order().lock() else {
        return;
    };
    #[cfg(debug_assertions)]
    {
        let new_set: HashSet<StableWindowKey> = order.iter().copied().collect();
        let dropped: Vec<String> = cached
            .iter()
            .filter(|key| !new_set.contains(key))
            .map(|key| key.window_id.to_string())
            .collect();
        if !dropped.is_empty() || cached.len() != order.len() {
            qol_runtime::probe!(
                "ORDER_PERSIST",
                "prev={} new={} dropped=[{}] head=[{}]",
                cached.len(),
                order.len(),
                dropped.join(" "),
                order
                    .iter()
                    .take(8)
                    .map(|key| key.window_id.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
    }
    *cached = order;
}

fn register_on_screen(parsed: &[CgWindow], state: &mut WindowEnumeration) -> HashMap<i32, bool> {
    let mut regular_cache: HashMap<i32, bool> = HashMap::new();
    for w in parsed {
        state.on_screen_ids.insert(w.id);
        if *regular_cache
            .entry(w.pid)
            .or_insert_with(|| is_regular_app(w.pid))
        {
            state.register_on_screen_pid(w.pid);
        }
    }
    regular_cache
}

fn filter_visible(parsed: Vec<CgWindow>, regular_cache: &HashMap<i32, bool>) -> Vec<CgWindow> {
    parsed
        .into_iter()
        .filter(|w| {
            if w.w < MIN_WINDOW_DIM || w.h < MIN_WINDOW_DIM {
                qol_runtime::probe!(
                    "FILTERED",
                    "reason=small wid={} app={:?} dims={}x{}",
                    w.id,
                    w.app_name,
                    w.w,
                    w.h
                );
                return false;
            }
            let is_regular = regular_cache[&w.pid];
            if !is_regular {
                qol_runtime::probe!(
                    "FILTERED",
                    "reason=irregular wid={} pid={} app={:?} policy={}",
                    w.id,
                    w.pid,
                    w.app_name,
                    super::process::app_policy_debug(w.pid)
                );
            }
            is_regular
        })
        .collect()
}

struct AxData<'a> {
    id_map: &'a HashMap<u32, AxWindowMeta>,
    all_meta: &'a [AxWindowMeta],
    accepted: usize,
}

impl AxData<'_> {
    fn has_data(&self) -> bool {
        !self.id_map.is_empty() || !self.all_meta.is_empty()
    }

    fn minimized_count(&self) -> usize {
        if !self.id_map.is_empty() {
            return self.id_map.values().filter(|w| w.is_minimized).count();
        }
        self.all_meta.iter().filter(|w| w.is_minimized).count()
    }
}

fn allowed_minimized_count(
    on_screen_count: usize,
    ax: &AxData,
    identity: Option<ProcessIdentity>,
    snapshot: &HashMap<ProcessIdentity, HashSet<u32>>,
    is_hidden: bool,
) -> usize {
    if ax.has_data() && on_screen_count != 0 {
        return ax.minimized_count();
    }
    let snap_count = identity
        .filter(|_| on_screen_count == 0)
        .and_then(|id| snapshot.get(&id))
        .map(|ids| ids.len())
        .filter(|c| *c > 0);
    if let Some(count) = snap_count {
        return count;
    }
    if ax.has_data() {
        if is_hidden {
            return ax.accepted.max(1);
        }
        return ax.minimized_count();
    }
    if ax.accepted > on_screen_count {
        return ax.accepted - on_screen_count;
    }
    if ax.accepted > 0 {
        return 0;
    }
    usize::MAX
}

fn detect_cross_space_pids(
    state: &WindowEnumeration,
    ax_cache: &AxCache,
    candidate_pids: &HashSet<i32>,
) -> HashSet<i32> {
    let mut result = HashSet::new();
    for &pid in candidate_pids {
        let Some(entry) = ax_cache.get(&pid) else {
            continue;
        };
        let Some((id_map, _, accepted)) = entry else {
            continue;
        };
        if id_map.is_empty() {
            continue;
        }
        if *accepted > state.on_screen_count(pid) {
            result.insert(pid);
        }
    }
    result
}

fn accept_cross_space(
    window: &CgWindow,
    cross_space_pids: &HashSet<i32>,
    hidden_pids: &HashSet<i32>,
    ax_cache: &AxCache,
) -> Option<String> {
    if hidden_pids.contains(&window.pid) {
        return None;
    }
    if !window.is_cross_space && !cross_space_pids.contains(&window.pid) {
        return None;
    }
    let ax_meta = match ax_cache.get(&window.pid) {
        Some(Some((id_map, _, _))) => id_map.get(&window.id),
        _ => None,
    };
    let Some(meta) = ax_meta else {
        return window.is_cross_space.then(|| window.title.clone());
    };
    if meta.is_minimized {
        return None;
    }
    Some(if meta.title.is_empty() {
        window.title.clone()
    } else {
        meta.title.clone()
    })
}

fn passes_ax_filter(
    window: &CgWindow,
    state: &WindowEnumeration,
    other_space_pids: &HashSet<i32>,
    hidden_pids: &HashSet<i32>,
    ax_cache: &AxCache,
) -> bool {
    let is_on_screen_pid = state.on_screen_pids.contains(&window.pid);
    let is_other_space_pid = other_space_pids.contains(&window.pid);
    let is_hidden = hidden_pids.contains(&window.pid);
    let Some(ax_result) = ax_cache.get(&window.pid) else {
        return false;
    };
    let Some((id_map, all_meta, _)) = ax_result else {
        return false;
    };
    if let Some(meta) = id_map.get(&window.id) {
        if !is_on_screen_pid {
            return meta.is_minimized || is_hidden;
        }
        if is_other_space_pid {
            return true;
        }
        return meta.is_minimized;
    }
    if !all_meta.is_empty() {
        return ax_title_match(window, all_meta, is_other_space_pid, is_hidden);
    }
    false
}

fn ax_title_match(
    window: &CgWindow,
    all_meta: &[AxWindowMeta],
    is_other_space: bool,
    is_hidden: bool,
) -> bool {
    let title_match = all_meta.iter().find(|m| m.title == window.title);
    if is_other_space {
        return title_match.is_some();
    }
    if let Some(meta) = title_match {
        return meta.is_minimized || is_hidden;
    }
    if all_meta.len() == 1 {
        return all_meta[0].is_minimized || is_hidden;
    }
    false
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
    window: &CgWindow,
    state: &WindowEnumeration,
    tracker: &mut KnownWindowTracker,
    ax_cache: &mut AxCache,
    is_hidden: bool,
) -> BudgetContext {
    tracker.identity_for_pid(window.pid);
    let ax = ax_cache
        .entry(window.pid)
        .or_insert_with(|| ax::ax_windows(window.pid));
    let ax_meta = ax
        .as_ref()
        .and_then(|(id_map, all_meta, _)| {
            id_map
                .get(&window.id)
                .or_else(|| all_meta.iter().find(|m| m.title == window.title))
        })
        .cloned();
    let ax_accepted = ax.as_ref().map_or(0, |(_, _, c)| *c);
    let (known_ids, allowed_count) =
        compute_allowed_count(window, state, tracker, ax_cache, is_hidden);
    BudgetContext {
        known_ids,
        ax_meta,
        ax_accepted,
        allowed_count,
    }
}

fn compute_allowed_count(
    window: &CgWindow,
    state: &WindowEnumeration,
    tracker: &KnownWindowTracker,
    ax_cache: &AxCache,
    is_hidden: bool,
) -> (Option<HashSet<u32>>, usize) {
    let on_screen_count = state.on_screen_count(window.pid);
    let known_ids = tracker
        .cached_identity(window.pid)
        .filter(|_| on_screen_count == 0)
        .and_then(|id| tracker.snapshot.get(&id))
        .cloned();
    let known_budget = known_ids.as_ref().map(|ids| ids.len()).filter(|c| *c > 0);
    if let Some(b) = known_budget {
        return (known_ids, b);
    }
    let count = match ax_cache.get(&window.pid) {
        Some(Some((id_map, all_meta, accepted))) => {
            let ax = AxData {
                id_map,
                all_meta,
                accepted: *accepted,
            };
            allowed_minimized_count(
                on_screen_count,
                &ax,
                tracker.cached_identity(window.pid),
                &tracker.snapshot,
                is_hidden,
            )
        }
        _ => usize::MAX,
    };
    (known_ids, count)
}

fn resolve_minimized_budget(
    ctx: &BudgetContext,
    window: &CgWindow,
    is_hidden: bool,
) -> Option<ResolvedWindow> {
    let ax_has = ctx.ax_meta.is_some();
    let ax_off_screen = ctx
        .ax_meta
        .as_ref()
        .is_some_and(|m| m.is_minimized || is_hidden);
    if ctx.known_ids.is_none() && ax_has && !ax_off_screen {
        return None;
    }
    if let Some(ids) = &ctx.known_ids {
        if !ids.is_empty() && !ax_has && ctx.ax_accepted == 0 && !ids.contains(&window.id) {
            return None;
        }
    }
    let title = ctx
        .ax_meta
        .as_ref()
        .filter(|m| !m.title.is_empty())
        .map(|m| m.title.clone())
        .unwrap_or_else(|| window.title.clone());
    Some(ResolvedWindow {
        title,
        allowed_count: ctx.allowed_count,
    })
}

pub(super) fn collect_off_screen_windows(
    off_screen: Vec<CgWindow>,
    include_minimized: bool,
    state: &mut WindowEnumeration,
    tracker: &mut KnownWindowTracker,
    ax_cache: &mut AxCache,
) {
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/enum] CG off-screen candidates: {}",
        off_screen.len()
    );

    let candidates = filter_minimized_candidates(off_screen, state);
    let candidate_pids = candidate_pids(&candidates);
    prefetch_missing_ax(&candidate_pids, ax_cache);
    let cross_space_pids = detect_cross_space_pids(state, ax_cache, &candidate_pids);
    let other_space_pids: HashSet<i32> = cross_space_pids
        .iter()
        .copied()
        .filter(|pid| state.on_screen_pids.contains(pid))
        .collect();
    let hidden_pids = detect_hidden_pids(&candidate_pids);
    let mut budget_counts: HashMap<i32, usize> = HashMap::new();
    #[cfg(debug_assertions)]
    let mut cross_accepted: Vec<String> = Vec::new();

    for window in candidates {
        if let Some(title) = accept_cross_space(&window, &cross_space_pids, &hidden_pids, ax_cache)
        {
            #[cfg(debug_assertions)]
            cross_accepted.push(format!("{}:{}", window.id, window.app_name));
            tracker.remember_window(window.pid, window.id);
            state.push_other_space(&window, title);
            continue;
        }
        if !include_minimized {
            continue;
        }
        if !passes_ax_filter(&window, state, &other_space_pids, &hidden_pids, ax_cache) {
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/enum] MINIMIZED skip (AX filter): wid={} app={:?}",
                window.id, window.app_name
            );
            continue;
        }
        let is_hidden = hidden_pids.contains(&window.pid);
        let Some(resolved) = try_accept_minimized(
            &window,
            state,
            tracker,
            ax_cache,
            &mut budget_counts,
            is_hidden,
        ) else {
            continue;
        };
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/enum] MINIMIZED accepted: wid={} app={:?} title={:?}",
            window.id, window.app_name, resolved.title
        );
        tracker.remember_window(window.pid, window.id);
        state.push_minimized(&window, resolved.title);
    }

    #[cfg(debug_assertions)]
    if !cross_accepted.is_empty() {
        qol_runtime::probe!(
            "CROSS_SPACE",
            "n={} head=[{}]",
            cross_accepted.len(),
            cross_accepted
                .iter()
                .take(6)
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/enum] total windows after minimized: {}",
        state.windows.len()
    );
}

fn candidate_pids(candidates: &[CgWindow]) -> HashSet<i32> {
    candidates.iter().map(|window| window.pid).collect()
}

fn detect_hidden_pids(candidate_pids: &HashSet<i32>) -> HashSet<i32> {
    candidate_pids
        .iter()
        .copied()
        .filter(|&pid| is_app_hidden(pid))
        .collect()
}

fn prefetch_missing_ax(pids: &HashSet<i32>, ax_cache: &mut AxCache) {
    let missing = pids
        .iter()
        .filter(|pid| !ax_cache.contains_key(pid))
        .copied()
        .collect::<HashSet<_>>();
    if missing.is_empty() {
        return;
    }
    ax_cache.extend(ax::prefetch_ax_parallel(missing));
}

fn filter_minimized_candidates(
    off_screen: Vec<CgWindow>,
    state: &WindowEnumeration,
) -> Vec<CgWindow> {
    let mut seen_ids = HashSet::new();
    let mut regular_cache: HashMap<i32, bool> = HashMap::new();
    off_screen
        .into_iter()
        .filter(|w| {
            if state.on_screen_ids.contains(&w.id) {
                return false;
            }
            if !seen_ids.insert(w.id) {
                return false;
            }
            if !w.has_title || w.w < 1.0 || w.h < 1.0 {
                return false;
            }
            *regular_cache
                .entry(w.pid)
                .or_insert_with(|| is_regular_app(w.pid))
        })
        .collect()
}

fn try_accept_minimized(
    window: &CgWindow,
    state: &WindowEnumeration,
    tracker: &mut KnownWindowTracker,
    ax_cache: &mut AxCache,
    budget_counts: &mut HashMap<i32, usize>,
    is_hidden: bool,
) -> Option<ResolvedWindow> {
    let ctx = build_budget_context(window, state, tracker, ax_cache, is_hidden);
    let resolved = resolve_minimized_budget(&ctx, window, is_hidden)?;
    if resolved.allowed_count == 0 {
        return None;
    }
    let count = budget_counts.entry(window.pid).or_insert(0);
    if *count >= resolved.allowed_count {
        return None;
    }
    *count += 1;
    Some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(pid: i32, start_time_us: u64, window_id: u32) -> StableWindowKey {
        StableWindowKey {
            identity: ProcessIdentity { pid, start_time_us },
            window_id,
        }
    }

    fn cg(id: u32, pid: i32) -> CgWindow {
        CgWindow {
            id,
            pid,
            app_name: "foo".to_string(),
            title: "bar".to_string(),
            has_title: true,
            is_onscreen: false,
            is_cross_space: false,
            x: 0.0,
            y: 0.0,
            w: 500.0,
            h: 400.0,
        }
    }

    fn cg_cross(id: u32, pid: i32) -> CgWindow {
        CgWindow {
            is_cross_space: true,
            ..cg(id, pid)
        }
    }

    fn ax_entry(
        windows: &[(u32, &str, bool)],
        accepted: usize,
    ) -> Option<(HashMap<u32, AxWindowMeta>, Vec<AxWindowMeta>, usize)> {
        let id_map = windows
            .iter()
            .map(|(id, title, minimized)| {
                (
                    *id,
                    AxWindowMeta {
                        title: title.to_string(),
                        is_minimized: *minimized,
                    },
                )
            })
            .collect();
        let all_meta = windows
            .iter()
            .map(|(_, title, minimized)| AxWindowMeta {
                title: title.to_string(),
                is_minimized: *minimized,
            })
            .collect();
        Some((id_map, all_meta, accepted))
    }

    #[test]
    fn cross_space_acceptance_lists_real_windows_only() {
        let cross: HashSet<i32> = [1].into_iter().collect();
        let no_cross: HashSet<i32> = HashSet::new();
        let hidden: HashSet<i32> = [1].into_iter().collect();
        let no_hidden: HashSet<i32> = HashSet::new();
        let ax_live: AxCache = [(1, ax_entry(&[(10, "win a", false)], 1))]
            .into_iter()
            .collect();
        let ax_minimized: AxCache = [(1, ax_entry(&[(10, "win a", true)], 1))]
            .into_iter()
            .collect();
        let ax_unknown_wid: AxCache = [(1, ax_entry(&[(11, "win b", false)], 1))]
            .into_iter()
            .collect();
        let ax_untitled: AxCache = [(1, ax_entry(&[(10, "", false)], 1))].into_iter().collect();

        type Case<'a> = (
            &'a str,
            &'a HashSet<i32>,
            &'a HashSet<i32>,
            &'a AxCache,
            Option<&'a str>,
        );
        let cases: &[Case] = &[
            (
                "ax-known live window",
                &cross,
                &no_hidden,
                &ax_live,
                Some("win a"),
            ),
            (
                "minimized goes to minimized flow",
                &cross,
                &no_hidden,
                &ax_minimized,
                None,
            ),
            ("hidden app excluded", &cross, &hidden, &ax_live, None),
            ("pid not cross-space", &no_cross, &no_hidden, &ax_live, None),
            (
                "cg-only phantom not in ax",
                &cross,
                &no_hidden,
                &ax_unknown_wid,
                None,
            ),
            (
                "empty ax title falls back to cg",
                &cross,
                &no_hidden,
                &ax_untitled,
                Some("bar"),
            ),
        ];
        for (name, cross_pids, hidden_pids, ax_cache, expected) in cases {
            assert_eq!(
                accept_cross_space(&cg(10, 1), cross_pids, hidden_pids, ax_cache).as_deref(),
                *expected,
                "case: {name}"
            );
        }
    }

    #[test]
    fn sls_flagged_windows_accepted_without_ax() {
        let no_cross: HashSet<i32> = HashSet::new();
        let hidden: HashSet<i32> = [1].into_iter().collect();
        let no_hidden: HashSet<i32> = HashSet::new();
        let ax_empty: AxCache = HashMap::new();
        let ax_empty_map: AxCache = [(1, ax_entry(&[], 0))].into_iter().collect();
        let ax_titled: AxCache = [(1, ax_entry(&[(10, "win a", false)], 1))]
            .into_iter()
            .collect();
        let ax_minimized: AxCache = [(1, ax_entry(&[(10, "win a", true)], 1))]
            .into_iter()
            .collect();

        type Case<'a> = (&'a str, &'a HashSet<i32>, &'a AxCache, Option<&'a str>);
        let cases: &[Case] = &[
            (
                "no ax entry falls back to cg",
                &no_hidden,
                &ax_empty,
                Some("bar"),
            ),
            (
                "empty ax id_map falls back to cg",
                &no_hidden,
                &ax_empty_map,
                Some("bar"),
            ),
            (
                "ax title wins when present",
                &no_hidden,
                &ax_titled,
                Some("win a"),
            ),
            (
                "ax-known minimized rejected",
                &no_hidden,
                &ax_minimized,
                None,
            ),
            ("hidden app excluded", &hidden, &ax_empty, None),
        ];
        for (name, hidden_pids, ax_cache, expected) in cases {
            assert_eq!(
                accept_cross_space(&cg_cross(10, 1), &no_cross, hidden_pids, ax_cache).as_deref(),
                *expected,
                "case: {name}"
            );
        }
    }

    #[test]
    fn cross_space_pid_detection_does_not_require_onscreen_presence() {
        let state = WindowEnumeration::default();
        let pids: HashSet<i32> = [1, 2, 3].into_iter().collect();
        let ax_cache: AxCache = [
            (1, ax_entry(&[(10, "a", false)], 1)),
            (2, ax_entry(&[], 0)),
            (3, None),
        ]
        .into_iter()
        .collect();

        let result = detect_cross_space_pids(&state, &ax_cache, &pids);
        assert!(result.contains(&1), "pid with ax windows and zero onscreen");
        assert!(!result.contains(&2), "pid with empty ax id_map");
        assert!(!result.contains(&3), "pid with failed ax read");
    }

    #[test]
    fn merge_stable_keys_keeps_non_focused_windows_in_prior_slots() {
        let a1 = key(10, 1, 101);
        let a2 = key(10, 1, 102);
        let a3 = key(10, 1, 103);
        let b1 = key(20, 2, 201);
        let c1 = key(30, 3, 301);

        let previous = vec![a1, b1, a2, c1, a3];
        let current = vec![a2, a1, a3, b1, c1];

        assert_eq!(
            merge_stable_keys(Some(a2), &previous, &current),
            vec![a2, a1, b1, c1, a3]
        );
    }

    #[test]
    fn merge_stable_keys_appends_new_windows_after_known_ones() {
        let a1 = key(10, 1, 101);
        let b1 = key(20, 2, 201);
        let c1 = key(30, 3, 301);
        let d1 = key(40, 4, 401);

        let previous = vec![a1, b1];
        let current = vec![b1, c1, a1, d1];

        assert_eq!(
            merge_stable_keys(Some(b1), &previous, &current),
            vec![b1, a1, c1, d1]
        );
    }

    #[test]
    fn keeps_stable_order_when_a_known_window_is_focused() {
        let a1 = key(10, 1, 101);
        let b1 = key(20, 2, 201);
        let c1 = key(30, 3, 301);

        assert!(!use_current_window_order(Some(b1), &[a1, c1, b1]));
    }

    #[test]
    fn uses_current_order_only_on_cold_start() {
        let a1 = key(10, 1, 101);
        let b1 = key(20, 2, 201);

        assert!(use_current_window_order(Some(a1), &[]));
        assert!(!use_current_window_order(Some(a1), &[a1, b1]));
    }

    #[test]
    fn keeps_stable_order_when_frontmost_is_unchanged() {
        let a1 = key(10, 1, 101);
        let b1 = key(20, 2, 201);
        let c1 = key(30, 3, 301);

        assert!(!use_current_window_order(Some(a1), &[a1, b1, c1]));
    }
}
