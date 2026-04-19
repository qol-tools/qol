use super::AltTabApp;
use crate::actions;
use crate::shared::layout::rendered_column_count;
use gpui::{Context, Window};

pub(crate) fn handle_key_down(
    this: &mut AltTabApp,
    event: &gpui::KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<AltTabApp>,
) {
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/input] key_down: key={:?} alt={} shift={}",
        event.keystroke.key, event.keystroke.modifiers.alt, event.keystroke.modifiers.shift,
    );
    match event.keystroke.key.as_str() {
        "escape" | "esc" => this.dismiss("key/escape", window, cx),
        "enter" => on_activate(this, window, cx),
        "w" => on_close(this, cx),
        "q" => on_quit(this, cx),
        "r" => on_minimize(this, cx),
        "tab" => on_tab(this, event.keystroke.modifiers.shift, cx),
        "backtab" => on_tab(this, true, cx),
        "right" | "arrowright" => on_arrow(this, |s, c| s.select_right(c), window, cx),
        "left" | "arrowleft" => on_arrow(this, |s, c| s.select_left(c), window, cx),
        "down" | "arrowdown" => on_arrow(this, |s, c| s.select_down(c), window, cx),
        "up" | "arrowup" => on_arrow(this, |s, c| s.select_up(c), window, cx),
        _ => {}
    }
}

fn on_activate(this: &mut AltTabApp, window: &mut Window, cx: &mut Context<AltTabApp>) {
    if this.delegate.read(cx).selected_index.is_none() {
        return;
    }
    this.delegate.update(cx, |s, _| s.activate_selected_target());
    this.dismiss("key/enter", window, cx);
}

fn on_close(this: &mut AltTabApp, cx: &mut Context<AltTabApp>) {
    let Some(win_id) = selected_window_id(this, cx) else {
        return;
    };
    actions::close_window(win_id);
    this.delegate.update(cx, |s, _| s.remove_window(win_id));
    cx.notify();
}

fn on_quit(this: &mut AltTabApp, cx: &mut Context<AltTabApp>) {
    let Some(win_id) = selected_window_id(this, cx) else {
        return;
    };
    let app_name = this
        .delegate
        .read(cx)
        .windows
        .iter()
        .find(|w| w.id == win_id)
        .map(|w| w.app_name.clone());
    actions::quit_app(win_id);
    if let Some(name) = app_name {
        this.delegate.update(cx, |s, _| s.remove_app_windows(&name));
    }
    cx.notify();
}

fn on_minimize(this: &mut AltTabApp, cx: &mut Context<AltTabApp>) {
    let Some(win_id) = selected_window_id(this, cx) else {
        return;
    };
    actions::minimize_window_by_id(win_id);
    this.delegate.update(cx, |s, _| s.mark_minimized(win_id));
    cx.notify();
}

fn on_tab(this: &mut AltTabApp, reverse: bool, cx: &mut Context<AltTabApp>) {
    this.delegate.update(cx, |s, _| {
        if reverse {
            s.select_prev();
        } else {
            s.select_next();
        }
    });
    cx.notify();
}

fn on_arrow(
    this: &mut AltTabApp,
    nav: impl FnOnce(&mut crate::picker::state::PickerState, usize),
    window: &Window,
    cx: &mut Context<AltTabApp>,
) {
    let cols = rendered_column_count(window, this.delegate.read(cx).windows.len());
    this.delegate.update(cx, |s, _| nav(s, cols));
    cx.notify();
}

fn selected_window_id(this: &AltTabApp, cx: &Context<AltTabApp>) -> Option<u32> {
    let state = this.delegate.read(cx);
    state
        .selected_index
        .and_then(|ix| state.windows.get(ix).map(|w| w.id))
}
