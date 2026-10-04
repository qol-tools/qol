mod about;
mod click_away;
mod controller;
mod details;
mod feedback;
mod files;
mod input;
pub(crate) mod keepalive;
mod layout;
mod menu;
mod platform;
mod render;
pub mod run;
mod sizing;
mod state;
mod tags;
mod trace;
mod transitions;
mod view;
mod window_host;

use std::sync::{mpsc, Arc};
use std::time::Duration;

use gpui::*;

use crate::discovery::details::AppFace;
use crate::discovery::entry_store::{BeforeTyping, EntryStore};
use crate::discovery::{PreloadedEntries, SharedEntries};

use layout::WINDOW_WIDTH;
use menu::MenuKind;
use state::LauncherState;

pub use input::key_to_input_char;

const BLUR_GUARD_MS: u64 = 400;
const TRAIL_DECAY_TICK: Duration = Duration::from_millis(20);
const LAUNCHER_APP_ID: &str = qol_conventions::launcher::APP_ID;
pub(crate) const LAUNCHER_WINDOW_TITLE: &str = qol_conventions::launcher::WINDOW_TITLE;

pub(crate) struct LauncherView {
    pub(super) state: LauncherState,
    pub(super) store: EntryStore,
    shared_entries: SharedEntries,
    last_entries_snapshot: Arc<PreloadedEntries>,
    pub(super) focus_handle: FocusHandle,
    dismiss_sub: Option<(Subscription, Subscription, Option<Task<()>>)>,
    pub(super) detail_scroll: ScrollHandle,
    pub(super) menu_scroll: ScrollHandle,
    menu_kind: Option<MenuKind>,
    pub(super) menu_selected: usize,
    details: details::DetailCache,
    last_layout: Vec<feedback::Placed>,
    transitions: transitions::Transitions,
    frame: Option<render::Frame>,
    leaving: Vec<render::Shown>,
    recent_stale: bool,
    trail_decay_task_running: bool,
    entry_watch_running: bool,
    pub(super) dismiss_requested: bool,
    dismiss_requested_from: &'static str,
    click_away_monitor: Option<click_away::Monitor>,
    click_away_arm: click_away::ArmState,
    input_region: Option<(i16, i16, u16, u16)>,
    pub(crate) is_showing: bool,
    pub(crate) showing_flag: Arc<std::sync::atomic::AtomicBool>,
    blur_guard: qol_gpui::ghost::BlurGuard,
    pub(crate) window_title: String,
    pub(crate) window_origin: Point<Pixels>,
    #[cfg(debug_assertions)]
    last_render_trace: Option<trace::RenderSignature>,
}

fn fresh_state(config: &crate::config::LauncherConfig) -> LauncherState {
    let mut state = LauncherState::new();
    state.escape_clears_text = config.escape_clears_text;
    state
}

fn before_typing(config: &crate::config::LauncherConfig) -> BeforeTyping {
    BeforeTyping {
        apps: config.most_used_apps,
        files: config.recent_files,
    }
}

impl LauncherView {
    pub(crate) fn new(title: String, shared: SharedEntries, cx: &mut Context<Self>) -> Self {
        let entries = shared
            .lock()
            .map(|g| g.entries.clone())
            .unwrap_or_else(|_| Arc::new(PreloadedEntries::empty()));
        let mut blur_guard = qol_gpui::ghost::BlurGuard::new();
        blur_guard.arm(Duration::from_millis(BLUR_GUARD_MS));
        let mut store = EntryStore::new(
            entries.app_entries.clone(),
            entries.file_entries.clone(),
            entries.flow_entries.clone(),
        );
        let config = crate::config::load_launcher_config();
        store.set_before_typing(before_typing(&config));
        Self {
            state: fresh_state(&config),
            store,
            shared_entries: shared,
            last_entries_snapshot: entries,
            focus_handle: cx.focus_handle(),
            dismiss_sub: None,
            detail_scroll: ScrollHandle::new(),
            menu_scroll: ScrollHandle::new(),
            menu_kind: None,
            menu_selected: 0,
            details: details::DetailCache::default(),
            last_layout: Vec::new(),
            transitions: transitions::Transitions::default(),
            frame: None,
            leaving: Vec::new(),
            recent_stale: true,
            trail_decay_task_running: false,
            entry_watch_running: false,
            dismiss_requested: false,
            dismiss_requested_from: "requested",
            click_away_monitor: None,
            click_away_arm: click_away::ArmState::default(),
            input_region: None,
            is_showing: true,
            showing_flag: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            blur_guard,
            window_title: title,
            window_origin: point(px(0.0), px(0.0)),
            #[cfg(debug_assertions)]
            last_render_trace: None,
        }
    }

    pub(crate) fn set_showing(&mut self, showing: bool) {
        self.is_showing = showing;
        self.showing_flag
            .store(showing, std::sync::atomic::Ordering::Relaxed);
        self.transitions.clear();
        self.frame = None;
        self.leaving.clear();
        if !showing {
            self.menu_kind = None;
            self.stop_click_away_monitor();
        }
    }

    pub(crate) fn hide_to_ghost(&mut self, _from: &'static str, _window: &mut Window) {
        trace::dismiss(self, _from);
        self.set_showing(false);
        qol_gpui::popup_window::release_input(&self.window_title);
        qol_gpui::ghost::dismiss_to_ghost(LAUNCHER_WINDOW_TITLE, &self.window_title);
        qol_gpui::popup_window::restore_composite(&self.window_title);
    }

    fn request_dismiss(&mut self, from: &'static str) {
        self.dismiss_requested = true;
        self.dismiss_requested_from = from;
    }

    pub(super) fn schedule_query_render(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    pub(crate) fn set_window_origin(&mut self, origin: Point<Pixels>) {
        self.window_origin = origin;
    }

    pub(crate) fn reset_for_show(&mut self) {
        #[cfg(debug_assertions)]
        if self.state.scroll_list.selected != 0 || !self.state.query.is_empty() {
            qol_runtime::probe!(
                "LAUNCHER_SEL_RESET",
                "reason=reset_for_show was={} q=\"{}\" title={}",
                self.state.scroll_list.selected,
                self.state.query.text(),
                self.window_title,
            );
        }
        let config = crate::config::load_launcher_config();
        self.state = fresh_state(&config);
        self.store.set_before_typing(before_typing(&config));
        self.details.forget_files();
        self.recent_stale = true;
        self.menu_kind = None;
        self.menu_selected = 0;
        self.menu_scroll = ScrollHandle::new();
        self.trail_decay_task_running = false;
        self.dismiss_requested = false;
        self.dismiss_requested_from = "requested";
        self.set_showing(true);
        self.blur_guard.arm(Duration::from_millis(BLUR_GUARD_MS));
        #[cfg(debug_assertions)]
        {
            self.last_render_trace = None;
        }
    }

    pub(super) fn face(&self, path: &std::path::Path) -> Option<&AppFace> {
        self.last_entries_snapshot.app_faces.get(path)
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
        self.store.replace_entries(
            fresh.app_entries.clone(),
            fresh.file_entries.clone(),
            fresh.flow_entries.clone(),
        );
        true
    }

    fn refresh_recent_files(&mut self, cx: &mut Context<Self>) {
        if self.recent_stale {
            self.recent_stale = false;
            self.load_recent_files(cx);
        }
    }

    fn ensure_click_away_monitor(&mut self, cx: &mut Context<Self>) {
        if !self.click_away_arm.should_start(self.is_showing) {
            return;
        }

        let (tx, rx) = mpsc::channel();
        let monitor = click_away::start(self.window_title.clone(), tx);
        let started = monitor.is_some();
        self.click_away_arm = self.click_away_arm.started(started);
        self.click_away_monitor = monitor;
        trace::click_away(
            &self.window_title,
            if started { "armed" } else { "unsupported" },
        );
        if started {
            self.spawn_click_away_task(rx, cx);
        }
    }

    fn spawn_click_away_task(&mut self, rx: mpsc::Receiver<()>, cx: &mut Context<Self>) {
        cx.spawn(move |this: WeakEntity<LauncherView>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let mut rx = rx;
                loop {
                    let (returned, received) = async_cx
                        .background_spawn(async move {
                            let received = rx.recv().is_ok();
                            (rx, received)
                        })
                        .await;
                    rx = returned;
                    if !received {
                        break;
                    }
                    let keep = this.update(&mut async_cx, |view, cx| {
                        if view.is_showing {
                            view.request_dismiss("click-away");
                            trace::click_away(&view.window_title, "fired");
                            cx.notify();
                        }
                        view.is_showing
                    });
                    if !matches!(keep, Ok(true)) {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn sync_input_region(&mut self, width: f32, height: f32, window: &Window) {
        let scale = window.scale_factor();
        let region = (
            ((WINDOW_WIDTH - width) / 2.0 * scale).round() as i16,
            0,
            (width * scale).round() as u16,
            (height * scale).ceil() as u16,
        );
        if self.input_region == Some(region) {
            return;
        }
        let (x, y, width, height) = region;
        if qol_gpui::popup_window::set_input_region_by_title(
            &self.window_title,
            x,
            y,
            width,
            height,
        ) {
            self.input_region = Some(region);
        }
    }

    fn stop_click_away_monitor(&mut self) {
        if self.click_away_monitor.is_some() {
            trace::click_away(&self.window_title, "stopped");
        }
        self.click_away_monitor = None;
        self.click_away_arm = self.click_away_arm.stopped();
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
                                log::debug!("entry watch: entries updated");
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
