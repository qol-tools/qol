mod controller;
mod input;
pub(crate) mod keepalive;
mod layout;
pub(crate) mod platform;
mod render;
pub mod run;
mod state;
mod view;
mod window_ops;
pub(crate) mod windows;

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::*;

use crate::discovery::entry_store::EntryStore;
use crate::discovery::PreloadedEntries;

use layout::HEADER_HEIGHT;
use state::LauncherState;

pub use input::key_to_input_char;

const BLUR_GUARD_MS: u64 = 400;
const TRAIL_DECAY_TICK: Duration = Duration::from_millis(20);
const LAUNCHER_APP_ID: &str = "qol-tray-launcher";

pub(crate) struct LauncherView {
    pub(super) state: LauncherState,
    pub(super) store: EntryStore,
    pub(super) focus_handle: FocusHandle,
    blur_sub: Option<Subscription>,
    activation_sub: Option<Subscription>,
    trail_decay_task_running: bool,
    pub(crate) is_showing: bool,
    blur_guard_until: Instant,
}

impl LauncherView {
    pub(crate) fn new(entries: Arc<PreloadedEntries>, cx: &mut Context<Self>) -> Self {
        Self {
            state: LauncherState::new(),
            store: EntryStore::new(entries.app_entries.clone(), entries.file_entries.clone()),
            focus_handle: cx.focus_handle(),
            blur_sub: None,
            activation_sub: None,
            trail_decay_task_running: false,
            is_showing: true,
            blur_guard_until: Instant::now() + Duration::from_millis(BLUR_GUARD_MS),
        }
    }

    pub(crate) fn set_showing(&mut self, showing: bool) {
        self.is_showing = showing;
    }

    pub(super) fn schedule_query_render(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    pub(crate) fn reset_for_show(&mut self) -> bool {
        let should_resize = (self.state.window_height - HEADER_HEIGHT).abs() > f32::EPSILON;
        self.state = LauncherState::new();
        self.trail_decay_task_running = false;
        self.set_showing(true);
        self.blur_guard_until = Instant::now() + Duration::from_millis(BLUR_GUARD_MS);
        should_resize
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
