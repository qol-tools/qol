use super::ax;
use super::events::AxEvent;
use crate::discovery::WindowInfo;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub(crate) type SharedWindowStore = Arc<Mutex<WindowStore>>;

#[derive(Default)]
pub(crate) struct WindowStore {
    windows: HashMap<u32, WindowInfo>,
    order: Vec<u32>,
    mru: Vec<u32>,
    last_focused: Option<u32>,
}

impl WindowStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn apply_event(&mut self, event: AxEvent) {
        match event {
            AxEvent::ApplicationActivated
            | AxEvent::FocusedWindowChanged
            | AxEvent::MainWindowChanged => self.refresh_focus(),
            AxEvent::WindowCreated | AxEvent::WindowDestroyed => {
                // Structural changes converge via background refresh_cache; no local mutation.
            }
            AxEvent::ApplicationHidden { pid } => self.set_pid_minimized(pid, true),
            AxEvent::ApplicationShown { pid } => self.set_pid_minimized(pid, false),
            AxEvent::WindowMiniaturized { pid } => self.sync_pid_minimized(pid),
            AxEvent::WindowDeminiaturized { pid } => self.sync_pid_minimized(pid),
        }
    }

    /// MRU-ordered snapshot of the current windows. The focused window is first,
    /// followed by the rest in most-recently-used order, then any windows the
    /// store knows about that have not been focused yet (in CG z-order).
    pub(crate) fn snapshot(&self) -> Vec<WindowInfo> {
        let mut seen = std::collections::HashSet::with_capacity(self.windows.len());
        let mut out = Vec::with_capacity(self.windows.len());
        for id in self.mru.iter().chain(self.order.iter()) {
            if !seen.insert(*id) {
                continue;
            }
            if let Some(w) = self.windows.get(id) {
                out.push(w.clone());
            }
        }
        out
    }

    /// Replace the store with the windows produced by a background refresh.
    /// Preserves caller-supplied order as the baseline z-order. MRU and
    /// last_focused are pruned to remain consistent with the new window set.
    pub(crate) fn replace_all(&mut self, windows: Vec<WindowInfo>) {
        self.order = windows.iter().map(|w| w.id).collect();
        self.windows = windows.into_iter().map(|w| (w.id, w)).collect();
        self.mru.retain(|id| self.windows.contains_key(id));
        if let Some(id) = self.last_focused {
            if !self.windows.contains_key(&id) {
                self.last_focused = None;
            }
        }
    }

    pub(crate) fn focused_window_id(&self) -> Option<u32> {
        self.last_focused
    }

    pub(crate) fn mru_order(&self) -> Vec<u32> {
        self.mru.clone()
    }

    fn refresh_focus(&mut self) {
        let Some(id) = ax::focused_window_id() else {
            return;
        };
        if self.last_focused == Some(id) {
            return;
        }
        self.last_focused = Some(id);
        self.mru.retain(|existing| *existing != id);
        self.mru.insert(0, id);
    }

    fn set_pid_minimized(&mut self, pid: i32, minimized: bool) {
        let Some(ids) = pid_window_ids(pid) else {
            return;
        };
        for id in ids {
            if let Some(w) = self.windows.get_mut(&id) {
                w.is_minimized = minimized;
            }
        }
    }

    fn sync_pid_minimized(&mut self, pid: i32) {
        let Some((id_map, _, _)) = ax::ax_windows(pid) else {
            return;
        };
        for (id, meta) in id_map {
            if let Some(w) = self.windows.get_mut(&id) {
                w.is_minimized = meta.is_minimized;
            }
        }
    }
}

fn pid_window_ids(pid: i32) -> Option<Vec<u32>> {
    let (id_map, _, _) = ax::ax_windows(pid)?;
    Some(id_map.into_keys().collect())
}

pub(crate) fn shared_window_store() -> SharedWindowStore {
    Arc::new(Mutex::new(WindowStore::new()))
}
