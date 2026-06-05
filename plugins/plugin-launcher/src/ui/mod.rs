mod controller;
mod input;
pub(crate) mod keepalive;
mod layout;
mod render;
pub mod run;
mod state;
mod view;
mod window_host;

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::*;

use crate::discovery::entry_store::EntryStore;
use crate::discovery::{PreloadedEntries, SharedEntries};

use layout::HEADER_HEIGHT;
use state::LauncherState;

pub use input::key_to_input_char;

const BLUR_GUARD_MS: u64 = 400;
const TRAIL_DECAY_TICK: Duration = Duration::from_millis(20);
const LAUNCHER_APP_ID: &str = "qol-tray-launcher";
pub(crate) const LAUNCHER_WINDOW_TITLE: &str = "qol-launcher";

pub(crate) struct LauncherView {
    pub(super) state: LauncherState,
    pub(super) store: EntryStore,
    shared_entries: SharedEntries,
    last_entries_snapshot: Arc<PreloadedEntries>,
    pub(super) focus_handle: FocusHandle,
    dismiss_sub: Option<(Subscription, Subscription, Option<Task<()>>)>,
    trail_decay_task_running: bool,
    entry_watch_running: bool,
    pub(super) dismiss_requested: bool,
    pub(crate) is_showing: bool,
    pub(crate) showing_flag: Arc<std::sync::atomic::AtomicBool>,
    blur_guard_until: Instant,
    pub(crate) window_title: String,
}

impl LauncherView {
    pub(crate) fn new(title: String, shared: SharedEntries, cx: &mut Context<Self>) -> Self {
        let entries = shared
            .lock()
            .map(|g| g.entries.clone())
            .unwrap_or_else(|_| Arc::new(PreloadedEntries::empty()));
        Self {
            state: LauncherState::new(),
            store: EntryStore::new(entries.app_entries.clone(), entries.file_entries.clone()),
            shared_entries: shared,
            last_entries_snapshot: entries,
            focus_handle: cx.focus_handle(),
            dismiss_sub: None,
            trail_decay_task_running: false,
            entry_watch_running: false,
            dismiss_requested: false,
            is_showing: true,
            showing_flag: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            blur_guard_until: Instant::now() + Duration::from_millis(BLUR_GUARD_MS),
            window_title: title,
        }
    }

    pub(crate) fn set_showing(&mut self, showing: bool) {
        self.is_showing = showing;
        self.showing_flag
            .store(showing, std::sync::atomic::Ordering::Relaxed);
    }

    pub(super) fn schedule_query_render(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    pub(crate) fn reset_for_show(&mut self) -> bool {
        let should_resize = (self.state.window_height - HEADER_HEIGHT).abs() > f32::EPSILON;
        self.state = LauncherState::new();
        self.trail_decay_task_running = false;
        self.dismiss_requested = false;
        self.set_showing(true);
        self.blur_guard_until = Instant::now() + Duration::from_millis(BLUR_GUARD_MS);
        should_resize
    }

    pub(crate) fn sync_entries_from_shared(&mut self) -> bool {
        let Ok(guard) = self.shared_entries.lock() else {
            return false;
        };
        if Arc::ptr_eq(&guard.entries, &self.last_entries_snapshot) {
            return false;
        }
        let fresh = guard.entries.clone();
        drop(guard);
        self.last_entries_snapshot = fresh.clone();
        self.store
            .replace_entries(fresh.app_entries.clone(), fresh.file_entries.clone());
        true
    }

    fn start_entry_watch(&mut self, cx: &mut Context<Self>) {
        if self.entry_watch_running {
            return;
        }
        self.entry_watch_running = true;
        cx.spawn(|this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                loop {
                    async_cx
                        .background_executor()
                        .timer(Duration::from_secs(2))
                        .await;
                    let should_continue = this
                        .update(&mut async_cx, |view, cx| {
                            if !view.is_showing {
                                view.entry_watch_running = false;
                                return false;
                            }
                            if view.sync_entries_from_shared() {
                                eprintln!("[launcher] entry watch: entries updated");
                                cx.notify();
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !should_continue {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn ensure_trail_decay_tick(&mut self, cx: &mut Context<Self>) {
        if self.trail_decay_task_running || self.state.decayed_momentum() == 0 {
            return;
        }

        self.trail_decay_task_running = true;
        cx.spawn(|this: WeakEntity<LauncherView>, cx: &mut AsyncApp| {
            let async_cx = cx.clone();
            async move {
                Self::trail_decay_loop(this, async_cx).await;
            }
        })
        .detach();
    }

    async fn trail_decay_loop(this: WeakEntity<Self>, mut async_cx: AsyncApp) {
        let mut last_level = u8::MAX;

        loop {
            async_cx.background_executor().timer(TRAIL_DECAY_TICK).await;
            if !Self::run_trail_decay_step(&this, &mut async_cx, &mut last_level) {
                break;
            }
        }
    }

    fn run_trail_decay_step(
        this: &WeakEntity<Self>,
        async_cx: &mut AsyncApp,
        last_level: &mut u8,
    ) -> bool {
        this.update(async_cx, |view, cx| {
            view.apply_trail_decay_update(last_level, cx)
        })
        .unwrap_or(false)
    }

    fn apply_trail_decay_update(&mut self, last_level: &mut u8, cx: &mut Context<Self>) -> bool {
        let level = self.state.decayed_momentum();
        if level == 0 {
            self.state.previous_selected = None;
            self.state.nav_direction = None;
            self.trail_decay_task_running = false;
            cx.notify();
            return false;
        }

        if level == *last_level {
            return true;
        }

        *last_level = level;
        cx.notify();
        true
    }
}
