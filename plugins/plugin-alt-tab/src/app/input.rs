use super::{AltTabApp, PICKER_VISIBLE};
use crate::shared::layout::rendered_column_count;
use crate::actions;
use crate::picker;
use gpui::{Context, Window};
use std::sync::atomic::Ordering;

fn selected_window_id(this: &AltTabApp, cx: &Context<AltTabApp>) -> Option<u32> {
    this.delegate
        .read(cx)
        .selected_index
        .and_then(|ix| this.delegate.read(cx).windows.get(ix).map(|w| w.id))
}

fn nav_cols(this: &AltTabApp, window: &Window, cx: &Context<AltTabApp>) -> usize {
    rendered_column_count(window, this.delegate.read(cx).windows.len())
}

pub(crate) fn handle_key_down(
    this: &mut AltTabApp,
    event: &gpui::KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<AltTabApp>,
) {
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/input] key_down: key={:?} key_char={:?} alt={} shift={} ctrl={} cmd={}",
        event.keystroke.key,
        event.keystroke.key_char,
        event.keystroke.modifiers.alt,
        event.keystroke.modifiers.shift,
        event.keystroke.modifiers.control,
        event.keystroke.modifiers.platform,
    );
    match event.keystroke.key.as_str() {
        "escape" | "esc" => {
            PICKER_VISIBLE.store(false, Ordering::Relaxed);
            picker::dismiss_picker(window);
        }
        "w" => {
            if let Some(win_id) = selected_window_id(this, cx) {
                actions::close_window(win_id);
                this.delegate.update(cx, |s, _cx| s.remove_window(win_id));
                cx.notify();
            }
        }
        "q" => {
            if let Some(win_id) = selected_window_id(this, cx) {
                let app_name = this
                    .delegate
                    .read(cx)
                    .windows
                    .iter()
                    .find(|w| w.id == win_id)
                    .map(|w| w.app_name.clone());
                actions::quit_app(win_id);
                if let Some(app_name) = app_name {
                    this.delegate.update(cx, |s, _cx| s.remove_app_windows(&app_name));
                }
                cx.notify();
            }
        }
        "r" => {
            if let Some(win_id) = selected_window_id(this, cx) {
                actions::minimize_window_by_id(win_id);
                this.delegate.update(cx, |s, _cx| s.mark_minimized(win_id));
                cx.notify();
            }
        }
        "enter" => {
            if this.delegate.read(cx).selected_index.is_some() {
                this.delegate.update(cx, |s, _cx| s.activate_selected(window));
            }
        }
        "tab" => {
            this.delegate.update(cx, |s, _cx| {
                if event.keystroke.modifiers.shift { s.select_prev(); } else { s.select_next(); }
            });
            cx.notify();
        }
        "backtab" => {
            this.delegate.update(cx, |s, _cx| s.select_prev());
            cx.notify();
        }
        "right" | "arrowright" => {
            let cols = nav_cols(this, window, cx);
            this.delegate.update(cx, |s, _cx| s.select_right(cols));
            cx.notify();
        }
        "left" | "arrowleft" => {
            let cols = nav_cols(this, window, cx);
            this.delegate.update(cx, |s, _cx| s.select_left(cols));
            cx.notify();
        }
        "down" | "arrowdown" => {
            let cols = nav_cols(this, window, cx);
            this.delegate.update(cx, |s, _cx| s.select_down(cols));
            cx.notify();
        }
        "up" | "arrowup" => {
            let cols = nav_cols(this, window, cx);
            this.delegate.update(cx, |s, _cx| s.select_up(cols));
            cx.notify();
        }
        _ => {}
    }
}
