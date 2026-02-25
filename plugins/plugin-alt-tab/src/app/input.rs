use super::{AltTabApp, PICKER_VISIBLE};
use crate::layout::rendered_column_count;
use crate::platform;
use gpui::{Context, Window};
use std::sync::atomic::Ordering;

pub(crate) fn handle_key_down(
    this: &mut AltTabApp,
    event: &gpui::KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<AltTabApp>,
) {
    match event.keystroke.key.as_str() {
        "escape" | "esc" => {
            PICKER_VISIBLE.store(false, Ordering::Relaxed);
            platform::dismiss_picker(window);
        }
        "enter" => {
            let win_id = this
                .delegate
                .read(cx)
                .selected_index
                .and_then(|ix| this.delegate.read(cx).windows.get(ix).map(|w| w.id));
            if win_id.is_some() {
                this.delegate.update(cx, |s, _cx| {
                    s.activate_selected(window);
                });
            }
        }
        "tab" => {
            this.delegate.update(cx, |s, _cx| {
                if event.keystroke.modifiers.shift {
                    s.select_prev();
                } else {
                    s.select_next();
                }
            });
            cx.notify();
        }
        "backtab" => {
            this.delegate.update(cx, |s, _cx| {
                s.select_prev();
            });
            cx.notify();
        }
        "right" | "arrowright" => {
            let total = this.delegate.read(cx).windows.len();
            let cols = rendered_column_count(window, total);
            this.delegate.update(cx, |s, _cx| {
                s.select_right(cols);
            });
            cx.notify();
        }
        "left" | "arrowleft" => {
            let total = this.delegate.read(cx).windows.len();
            let cols = rendered_column_count(window, total);
            this.delegate.update(cx, |s, _cx| {
                s.select_left(cols);
            });
            cx.notify();
        }
        "down" | "arrowdown" => {
            let total = this.delegate.read(cx).windows.len();
            let cols = rendered_column_count(window, total);
            this.delegate.update(cx, |s, _cx| {
                s.select_down(cols);
            });
            cx.notify();
        }
        "up" | "arrowup" => {
            let total = this.delegate.read(cx).windows.len();
            let cols = rendered_column_count(window, total);
            this.delegate.update(cx, |s, _cx| {
                s.select_up(cols);
            });
            cx.notify();
        }
        _ => {}
    }
}
