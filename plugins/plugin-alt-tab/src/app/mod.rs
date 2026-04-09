mod input;
mod live_preview;
mod render;

use crate::config::ActionMode;
use crate::picker;
use crate::picker::create::PickerInit;
use crate::picker::gather::GatheredWindows;
use crate::picker::state::PickerState;
use crate::IconMap;
use gpui::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(crate) static PICKER_VISIBLE: AtomicBool = AtomicBool::new(false);

const ALT_POLL_INTERVAL_MS: u64 = 50;
const ALT_POLL_ARM_TIMEOUT_MS: u64 = 200;

pub(crate) struct AltTabApp {
    pub(crate) delegate: Entity<PickerState>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) action_mode: ActionMode,
    pub(crate) alt_was_held: bool,
    pub(crate) _alt_poll_task: Option<Task<()>>,
    _live_preview_task: Option<Task<()>>,
    _focus_out_sub: gpui::Subscription,
}

impl AltTabApp {
    pub(crate) fn new(init: PickerInit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let should_cycle = init.cycle_on_open && init.windows.len() >= 2;
        let action_mode = init.action_mode.clone();
        let delegate = cx.new(|_| PickerState::from_init(init));

        if should_cycle {
            delegate.update(cx, |s, _| s.select_next());
        }

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/hold] AltTabApp::new: action_mode={:?}",
            action_mode
        );

        let mut app = Self {
            _focus_out_sub: subscribe_focus_out(&focus_handle, window, cx),
            _live_preview_task: None,
            delegate,
            focus_handle,
            action_mode: action_mode.clone(),
            alt_was_held: true,
            _alt_poll_task: None,
        };

        if action_mode == ActionMode::HoldToSwitch {
            app.start_alt_poll(window.to_async(cx).window_handle(), cx);
        }

        app
    }

    pub(crate) fn apply_reuse(
        &mut self,
        req: &crate::picker::ReuseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.reposition_if_needed(req, window) {
            return false;
        }
        self.apply_reuse_config(req, window, cx);
        self.sync_alt_poll(window, cx);
        self.apply_reuse_windows(req, cx);
        true
    }

    fn reposition_if_needed(&self, req: &crate::picker::ReuseRequest, window: &Window) -> bool {
        let current_origin = window.window_bounds().get_bounds().origin;
        let reposition_needed = picker::origin_diverged(current_origin, req.layout.bounds.origin);
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/hold] reuse path (poll_task={}) — reset={} reposition_needed={}",
            self._alt_poll_task.is_some(),
            req.config.reset_selection_on_open,
            reposition_needed,
        );
        if !reposition_needed {
            return true;
        }
        picker::platform::reposition_picker_window(
            req.layout.bounds.origin.x.to_f64(),
            req.layout.bounds.origin.y.to_f64(),
        )
    }

    /// Apply config changes to a reused picker window. Handles window-level
    /// properties (background appearance, shadow) that survive across opens,
    /// plus delegate-level config (colors, labels, hotkey hints).
    fn apply_reuse_config(
        &mut self,
        req: &crate::picker::ReuseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_transparent = self.delegate.read(cx).transparent_background;
        let now_transparent = req.config.display.transparent_background;
        if was_transparent != now_transparent {
            let appearance = if now_transparent {
                WindowBackgroundAppearance::Transparent
            } else {
                WindowBackgroundAppearance::Opaque
            };
            window.set_background_appearance(appearance);
        }
        if now_transparent {
            picker::platform::disable_window_shadow();
        }

        let (card_color, card_opacity) = crate::picker::resolve_card_bg(&req.config.display);
        self.action_mode = req.config.action_mode.clone();
        self.alt_was_held = true;
        self.delegate.update(cx, |s, _| {
            s.apply_config(req.config, card_color, card_opacity)
        });
    }

    fn sync_alt_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.action_mode == ActionMode::HoldToSwitch {
            self.start_alt_poll(window.window_handle(), cx);
            return;
        }
        self._alt_poll_task = None;
    }

    fn apply_reuse_windows(&mut self, req: &crate::picker::ReuseRequest, cx: &mut Context<Self>) {
        self.apply_gathered(req.gathered, req.config.reset_selection_on_open, cx);
        if req.config.open_behavior != crate::config::OpenBehavior::CycleOnce {
            return;
        }
        if !req.config.reset_selection_on_open {
            return;
        }
        if req.gathered.windows.len() < 2 {
            return;
        }
        self.delegate.update(cx, |s, _| s.select_next());
    }

    fn apply_gathered(&mut self, gathered: &GatheredWindows, reset: bool, cx: &mut Context<Self>) {
        self.delegate.update(cx, |state, cx| {
            state.set_windows(gathered.windows.clone(), reset);
            if !gathered.previews.is_empty() {
                state.live_previews = gathered.previews.clone();
            }
            if !gathered.icons.is_empty() {
                state.icon_cache = gathered.icons.clone();
            }
            cx.notify();
        });
        cx.notify();
    }

    pub(crate) fn update_icons(&mut self, icons: IconMap, cx: &mut Context<Self>) {
        self.delegate
            .update(cx, |state, _| state.insert_icons(icons));
        cx.notify();
    }

    pub(crate) fn ensure_live_preview(&mut self, cx: &mut Context<Self>) {
        if self._live_preview_task.is_some() {
            return;
        }
        self._live_preview_task = Some(live_preview::spawn(self.delegate.clone(), cx));
    }

    pub(crate) fn start_alt_poll(
        &mut self,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let delegate = self.delegate.clone();
        self.alt_was_held = true;
        self._alt_poll_task = Some(cx.spawn(move |this, cx: &mut AsyncApp| {
            alt_poll_loop(this, delegate, window_handle, cx.clone())
        }));
    }
}

async fn alt_poll_loop(
    this: WeakEntity<AltTabApp>,
    delegate: Entity<PickerState>,
    window_handle: AnyWindowHandle,
    mut cx: AsyncApp,
) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/hold] modifier poll task started");
    let mut saw_modifier = picker::is_modifier_held();
    cx.background_executor()
        .timer(Duration::from_millis(50))
        .await;
    let mut arm_wait_ms = 0;

    loop {
        cx.background_executor()
            .timer(Duration::from_millis(ALT_POLL_INTERVAL_MS))
            .await;
        if picker::is_modifier_held() {
            saw_modifier = true;
            continue;
        }
        if !saw_modifier {
            arm_wait_ms += ALT_POLL_INTERVAL_MS;
            if arm_wait_ms < ALT_POLL_ARM_TIMEOUT_MS {
                continue;
            }
            break;
        }
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/hold] Alt released — activating selected");
        let _ = cx.update_window(window_handle, |_, window, cx| {
            delegate.update(cx, |s, _| s.activate_selected(window));
        });
        break;
    }

    let _ = cx.update(|cx| {
        if let Some(entity) = this.upgrade() {
            entity.update(cx, |app, _| {
                app._alt_poll_task = None;
            });
        }
    });
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/hold] modifier poll task ended");
}

fn subscribe_focus_out(
    handle: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<AltTabApp>,
) -> gpui::Subscription {
    cx.on_focus_out(handle, window, |this, _event, window, _cx| {
        if this.action_mode == ActionMode::HoldToSwitch {
            return;
        }
        PICKER_VISIBLE.store(false, Ordering::Relaxed);
        picker::dismiss_picker(window);
    })
}

impl Focusable for AltTabApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
