pub(crate) mod alt_poll;
mod input;
mod live_preview;
mod render;

use crate::config::{ActionMode, LabelConfig};
use crate::picker::state::PickerState;
use crate::discovery::WindowInfo;
use crate::picker;
use crate::{IconMap, PreviewMap};
use gpui::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static PICKER_VISIBLE: AtomicBool = AtomicBool::new(false);

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
    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        action_mode: ActionMode,
        initial_windows: Vec<WindowInfo>,
        label_config: LabelConfig,
        transparent_background: bool,
        card_bg_color: u32,
        card_bg_opacity: f32,
        show_debug_overlay: bool,
        show_hotkey_hints: bool,
        cycle_on_open: bool,
        initial_previews: PreviewMap,
        icon_cache: IconMap,
    ) -> Self {
        let win_delegate = PickerState::new_with_previews(
            initial_windows.clone(),
            label_config,
            transparent_background,
            card_bg_color,
            card_bg_opacity,
            show_debug_overlay,
            show_hotkey_hints,
            initial_previews,
            icon_cache,
        );
        let delegate = cx.new(|_cx| win_delegate);

        if cycle_on_open && initial_windows.len() >= 2 {
            delegate.update(cx, |s, _| s.select_next());
        }

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        let gpui_window_handle = window.to_async(cx).window_handle();

        // Focus-out subscription for Sticky mode: dismiss picker when focus leaves.
        let focus_out_sub = cx.on_focus_out(
            &focus_handle,
            window,
            |this, _event, window, _cx| {
                if this.action_mode != ActionMode::HoldToSwitch {
                    PICKER_VISIBLE.store(false, Ordering::Relaxed);
                    picker::dismiss_picker(window);
                }
            },
        );

        let live_preview_task = live_preview::spawn(delegate.clone(), cx);

        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/hold] AltTabApp::new: action_mode={:?}, alt_was_held=true (assumed)",
            action_mode
        );

        let mut app = Self {
            delegate,
            focus_handle,
            action_mode: action_mode.clone(),
            alt_was_held: true,
            _alt_poll_task: None,
            _live_preview_task: Some(live_preview_task),
            _focus_out_sub: focus_out_sub,
        };

        if action_mode == ActionMode::HoldToSwitch {
            alt_poll::start(&mut app, gpui_window_handle, cx);
        }

        app
    }

    pub(crate) fn apply_cached_windows(
        &mut self,
        windows: Vec<WindowInfo>,
        reset_selection: bool,
        previews: PreviewMap,
        icons: IconMap,
        cx: &mut Context<Self>,
    ) {
        self.delegate.update(cx, |state, cx| {
            state.set_windows(windows, reset_selection);
            if !previews.is_empty() {
                state.live_previews = previews;
            }
            if !icons.is_empty() {
                state.icon_cache = icons;
            }
            cx.notify();
        });
        cx.notify();
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
        self.apply_reuse_config(req, cx);
        self.sync_alt_poll(window, cx);
        self.apply_reuse_windows(req, cx);
        true
    }

    fn reposition_if_needed(&self, req: &crate::picker::ReuseRequest) -> bool {
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/hold] reuse path (poll_task={}) — reset={} monitor_changed={}",
            self._alt_poll_task.is_some(), req.config.reset_selection_on_open, req.layout.monitor_changed,
        );
        if !req.layout.monitor_changed {
            return true;
        }
        picker::platform::reposition_picker_window(
            req.layout.bounds.origin.x.to_f64(), req.layout.bounds.origin.y.to_f64(),
        )
    }

    fn apply_reuse_config(&mut self, req: &crate::picker::ReuseRequest, cx: &mut Context<Self>) {
        let (card_color, card_opacity) = crate::picker::resolve_card_bg(&req.config.display);
        self.action_mode = req.config.action_mode.clone();
        self.alt_was_held = true;
        self.delegate.update(cx, |s, _| s.apply_config(req.config, card_color, card_opacity));
    }

    fn sync_alt_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.action_mode {
            ActionMode::HoldToSwitch => self.start_alt_poll(window.window_handle(), cx),
            _ => self._alt_poll_task = None,
        }
    }

    fn apply_reuse_windows(&mut self, req: &crate::picker::ReuseRequest, cx: &mut Context<Self>) {
        self.apply_cached_windows(
            req.gathered.windows.clone(), req.config.reset_selection_on_open,
            req.gathered.previews.clone(), req.gathered.icons.clone(), cx,
        );
        if req.config.open_behavior == crate::config::OpenBehavior::CycleOnce
            && req.config.reset_selection_on_open
            && req.gathered.windows.len() >= 2
        {
            self.delegate.update(cx, |s, _| s.select_next());
        }
    }

    pub(crate) fn update_icons(&mut self, icons: IconMap, cx: &mut Context<Self>) {
        self.delegate.update(cx, |state, _| state.insert_icons(icons));
        cx.notify();
    }

    pub(crate) fn start_alt_poll(
        &mut self,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        alt_poll::start(self, window_handle, cx);
    }
}

impl Focusable for AltTabApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
