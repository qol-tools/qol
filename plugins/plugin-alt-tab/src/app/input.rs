use super::AltTabApp;
use crate::actions;
use crate::shared::layout::picker_layout;
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
        "w" => on_close(this, window, cx),
        "q" => on_quit(this, window, cx),
        "r" => on_minimize(this, window, cx),
        "tab" => on_tab(this, event.keystroke.modifiers.shift, cx),
        "backtab" => on_tab(this, true, cx),
        "right" | "arrowright" => {
            on_arrow(this, "arrow-right", |s, c| s.select_right(c), window, cx)
        }
        "left" | "arrowleft" => on_arrow(this, "arrow-left", |s, c| s.select_left(c), window, cx),
        "down" | "arrowdown" => on_arrow(this, "arrow-down", |s, c| s.select_down(c), window, cx),
        "up" | "arrowup" => on_arrow(this, "arrow-up", |s, c| s.select_up(c), window, cx),
        _ => {}
    }
}

fn on_activate(this: &mut AltTabApp, window: &mut Window, cx: &mut Context<AltTabApp>) {
    if this.delegate.read(cx).selected_index.is_none() {
        return;
    }
    this.dismiss("key/enter", window, cx);
    this.delegate
        .update(cx, |s, _| s.activate_selected_target());
}

fn on_close(this: &mut AltTabApp, window: &mut Window, cx: &mut Context<AltTabApp>) {
    let Some(win_id) = selected_window_id(this, cx) else {
        return;
    };
    actions::close_window(win_id);
    this.delegate
        .update(cx, |s, ctx| s.remove_window(win_id, ctx, Some(window)));
    cx.notify();
}

fn on_quit(this: &mut AltTabApp, window: &mut Window, cx: &mut Context<AltTabApp>) {
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
        this.delegate
            .update(cx, |s, ctx| s.remove_app_windows(&name, ctx, Some(window)));
    }
    cx.notify();
}

fn on_minimize(this: &mut AltTabApp, window: &mut Window, cx: &mut Context<AltTabApp>) {
    let Some(win_id) = selected_window_id(this, cx) else {
        return;
    };
    actions::minimize_window_by_id(win_id);
    this.delegate
        .update(cx, |s, ctx| s.mark_minimized(win_id, ctx, Some(window)));
    cx.notify();
}

fn on_tab(this: &mut AltTabApp, reverse: bool, cx: &mut Context<AltTabApp>) {
    let from = this.delegate.read(cx).selected_index;
    this.delegate.update(cx, |s, _| {
        if reverse {
            s.select_prev();
        } else {
            s.select_next();
        }
    });
    this.mark_cycle(if reverse { "shift-tab" } else { "tab" }, from);
    cx.notify();
}

fn on_arrow(
    this: &mut AltTabApp,
    method: &'static str,
    nav: impl FnOnce(&mut crate::picker::state::PickerState, usize),
    _window: &Window,
    cx: &mut Context<AltTabApp>,
) {
    let state = this.delegate.read(cx);
    let layout = picker_layout(
        state.windows.len().max(1),
        state.max_columns,
        state.layout_budget,
        state.show_hotkey_hints,
        state.card_scale,
    );
    let from = state.selected_index;
    let count = state.windows.len();
    let scale = state.card_scale;
    let budget = state.layout_budget;
    this.delegate.update(cx, |s, _| nav(s, layout.columns));
    let to = this.delegate.read(cx).selected_index;
    qol_runtime::probe!(
        "NAV_GRID",
        "method={} from={:?} to={:?} cols={} count={} scale={} budget={:?}",
        method,
        from,
        to,
        layout.columns,
        count,
        scale,
        budget,
    );
    this.mark_cycle(method, from);
    cx.notify();
}

fn selected_window_id(this: &AltTabApp, cx: &Context<AltTabApp>) -> Option<u32> {
    let state = this.delegate.read(cx);
    state
        .selected_index
        .and_then(|ix| state.windows.get(ix).map(|w| w.id))
}
