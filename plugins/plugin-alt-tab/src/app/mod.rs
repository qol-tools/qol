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
use std::time::{Duration, Instant};

pub(crate) static PICKER_VISIBLE: AtomicBool = AtomicBool::new(false);

// PopUp windows on X11 fire one spurious blur right after creation; absorb only the first.
const BLUR_GUARD_MS: u64 = 250;

pub(crate) struct AltTabApp {
    pub(crate) delegate: Entity<PickerState>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) action_mode: ActionMode,
    pub(crate) alt_was_held: bool,
    pub(crate) blur_guard_until: Instant,
    pub(crate) blur_guard_armed: bool,
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
            blur_guard_until: Instant::now() + Duration::from_millis(BLUR_GUARD_MS),
            blur_guard_armed: true,
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
        if !self.reposition_if_needed(req) {
            return false;
        }
        self.apply_reuse_config(req, window, cx);
        self.sync_alt_poll(window, cx);
        self.apply_reuse_windows(req, cx);
        true
    }

    fn reposition_if_needed(&self, req: &crate::picker::ReuseRequest) -> bool {
        let reposition_needed = req.layout.monitor_changed;
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
        self.blur_guard_until = Instant::now() + Duration::from_millis(BLUR_GUARD_MS);
        self.blur_guard_armed = true;
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
            alt_release_check(this, delegate, window_handle, cx.clone())
        }));
    }
}

// on_modifiers_changed drives the common case, but it can be lost when the picker isn't yet
// key on reuse. Poll CGEventSource every ALT_POLL_INTERVAL_MS as a ground-truth fallback.
const ALT_POLL_INTERVAL_MS: u64 = 30;

async fn alt_release_check(
    this: WeakEntity<AltTabApp>,
    delegate: Entity<PickerState>,
    window_handle: AnyWindowHandle,
    mut cx: AsyncApp,
) {
    let executor = cx.background_executor().clone();
    loop {
        if this.upgrade().is_none() {
            return;
        }
        if !PICKER_VISIBLE.load(Ordering::Relaxed) {
            return;
        }
        if !picker::is_modifier_held() {
            break;
        }
        executor
            .timer(Duration::from_millis(ALT_POLL_INTERVAL_MS))
            .await;
    }
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/hold] Alt released via poll — activating selection");
    let weak = this.clone();
    let _ = cx.update_window(window_handle, move |_, window, cx| {
        delegate.update(cx, |s, _| s.activate_selected_target());
        if let Some(entity) = weak.upgrade() {
            entity.update(cx, |app, cx| app.dismiss("alt-release/poll", window, cx));
        }
    });
}

fn subscribe_focus_out(
    handle: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<AltTabApp>,
) -> gpui::Subscription {
    cx.on_focus_out(handle, window, |this, _event, window, cx| {
        // Absorb only the first post-create blur; later blurs are real dismiss requests.
        if this.blur_guard_armed && Instant::now() < this.blur_guard_until {
            this.blur_guard_armed = false;
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/blur] absorbed spurious post-create blur");
            return;
        }
        this.dismiss("focus-out", window, cx);
    })
}

impl AltTabApp {
    pub(crate) fn dismiss(
        &mut self,
        _source: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/dismiss] from={}", _source);
        self._alt_poll_task = None;
        self.blur_guard_armed = false;
        PICKER_VISIBLE.store(false, Ordering::Relaxed);
        picker::dismiss_picker(window);
        cx.notify();
    }
}

impl Focusable for AltTabApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
