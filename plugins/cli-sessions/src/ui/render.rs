use std::sync::LazyLock;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, ClickEvent, Context, CursorStyle, FontWeight, KeyDownEvent,
    KeyUpEvent, MouseButton, SharedString, Window,
};
use qol_gpui::surface::{DragGestureState, PanelDragArea};
use qol_gpui::theme::{cli_sessions_runtime, CliSessionsPalette};
use qol_gpui::WindowBar;
use qol_terminal_sessions::SessionId;

use crate::session::registry::{meaningful_name, SessionState};
use crate::session::status::Status;
use crate::session::tool::Tool;
use crate::ui::SessionsView;

const HIDE_BUTTON_REASON: &str = "hide-button";
const ESCAPE_REASON: &str = "escape";
const STRIP_ESCAPE_REASON: &str = "strip-escape";

static CURRENT_PALETTE: LazyLock<CliSessionsPalette> = LazyLock::new(cli_sessions_runtime);

fn current_palette() -> &'static CliSessionsPalette {
    &CURRENT_PALETTE
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn format_elapsed(since: u64) -> String {
    let secs = now_secs().saturating_sub(since);
    if secs < 5 {
        "now".to_string()
    } else if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

fn status_color(status: Status) -> u32 {
    let palette = current_palette();
    match status {
        Status::NeedsYou => palette.needs_you,
        Status::YourTurn => palette.your_turn,
        Status::Working => palette.working,
        Status::Service => palette.service,
        Status::Unknown => palette.unknown,
        Status::Acknowledged => palette.unknown,
    }
}

fn tint_color(status: Status) -> u32 {
    let palette = current_palette();
    match status {
        Status::NeedsYou => palette.needs_you_tint_rgba,
        Status::YourTurn => palette.your_turn_tint_rgba,
        Status::Working => palette.working_tint_rgba,
        Status::Service => palette.service_tint_rgba,
        Status::Unknown => palette.transparent_rgba,
        Status::Acknowledged => palette.transparent_rgba,
    }
}

fn status_glyph(status: Status) -> &'static str {
    match status {
        Status::NeedsYou => "!",
        Status::Working => "\u{25CF}",
        Status::Service => "\u{25CF}",
        Status::Unknown | Status::Acknowledged => "\u{00B7}",
        Status::YourTurn => "",
    }
}

fn summary_groups(rows: &[SessionState]) -> Vec<(u32, usize)> {
    let mut counts = [0usize; 5];
    for r in rows {
        match r.status {
            Status::NeedsYou => counts[0] += 1,
            Status::YourTurn => counts[1] += 1,
            Status::Working => counts[2] += 1,
            Status::Service => counts[3] += 1,
            Status::Unknown | Status::Acknowledged => counts[4] += 1,
        }
    }
    let colors = [
        status_color(Status::NeedsYou),
        status_color(Status::YourTurn),
        status_color(Status::Working),
        status_color(Status::Service),
        status_color(Status::Unknown),
    ];
    colors
        .into_iter()
        .zip(counts)
        .filter(|(_, n)| *n > 0)
        .collect()
}

fn meta_value(s: &SessionState) -> String {
    format_elapsed(s.last_activity)
}

fn summary_groups_el(rows: &[SessionState]) -> impl IntoElement {
    let palette = current_palette();
    div()
        .flex()
        .items_center()
        .gap(px(9.0))
        .children(summary_groups(rows).into_iter().map(|(color, count)| {
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(rgb(color)))
                .child(
                    div()
                        .text_color(rgb(palette.text_secondary))
                        .text_size(px(11.0))
                        .child(format!("{count}")),
                )
        }))
}

fn accepts_activation_click(event: &ClickEvent) -> bool {
    matches!(event, ClickEvent::Mouse(_))
}

fn header_button(
    id: &'static str,
    glyph: &'static str,
    activate: fn(&mut SessionsView, &mut Window, &mut Context<SessionsView>),
    cx: &mut Context<SessionsView>,
) -> impl IntoElement {
    let palette = current_palette();
    div()
        .id(id)
        .focusable()
        .tab_stop(true)
        .w(px(24.0))
        .h(px(24.0))
        .rounded_md()
        .border_1()
        .border_color(rgba(palette.transparent_rgba))
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(palette.text_secondary))
        .text_size(px(13.0))
        .cursor(CursorStyle::PointingHand)
        .hover(|style| style.bg(rgba(palette.keycap_bg_rgba)))
        .in_focus(|style| style.border_color(rgb(palette.selection_border)))
        .on_mouse_down(MouseButton::Left, |_, _, app| app.stop_propagation())
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            if accepts_activation_click(event) {
                activate(this, window, cx);
                cx.stop_propagation();
            }
        }))
        .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
            if matches!(ev.keystroke.key.as_str(), "enter" | "space")
                && this.key_repeat_guard(&ev.keystroke.key)
            {
                activate(this, window, cx);
                cx.stop_propagation();
            }
        }))
        .child(glyph)
}

fn header(rows: &[SessionState], cx: &mut Context<SessionsView>) -> impl IntoElement {
    let palette = current_palette();
    WindowBar::new("CLI SESSIONS")
        .background(palette.chrome_bg)
        .border(palette.divider)
        .title_color(palette.text_heading)
        .child(summary_groups_el(rows))
        .child(header_button(
            "collapse-panel-button",
            "\u{2581}",
            |this, window, cx| {
                this.collapse_panel(window);
                cx.notify();
            },
            cx,
        ))
        .child(header_button(
            "hide-panel-button",
            "\u{00D7}",
            |this, _window, cx| {
                this.dismiss_with_reason(HIDE_BUTTON_REASON);
                cx.notify();
            },
            cx,
        ))
}

fn empty_state() -> impl IntoElement {
    let palette = current_palette();
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .px(px(24.0))
        .child(
            div()
                .text_color(rgb(palette.text_heading))
                .text_size(px(13.0))
                .child("No CLI sessions found"),
        )
        .child(
            div()
                .text_color(rgb(palette.text_muted))
                .text_size(px(11.0))
                .child("Open a CLI in kitty, then open this panel again."),
        )
}

fn key_hint(key: &'static str, label: &'static str) -> impl IntoElement {
    let palette = current_palette();
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .text_color(rgb(palette.text_faint))
        .text_size(px(10.0))
        .child(
            div()
                .text_color(rgb(palette.text_heading))
                .text_size(px(9.0))
                .bg(rgba(palette.keycap_bg_rgba))
                .border_1()
                .border_color(rgb(palette.border))
                .rounded(px(4.0))
                .px(px(5.0))
                .py(px(1.0))
                .child(key),
        )
        .child(label)
}

fn footer() -> impl IntoElement {
    let palette = current_palette();
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .w_full()
        .px(px(11.0))
        .py(px(7.0))
        .bg(rgb(palette.chrome_bg))
        .border_t_1()
        .border_color(rgb(palette.divider))
        .child(key_hint("\u{2191}\u{2193}", "move"))
        .child(key_hint("\u{2190}\u{2192}", "cycle"))
        .child(key_hint("\u{23CE}", "jump"))
        .child(key_hint("a", "ack"))
        .child(key_hint("esc", "close"))
}

fn identity_line(s: &SessionState) -> impl IntoElement {
    let palette = current_palette();
    let branch = s.branch.clone().unwrap_or_default();
    let label = meaningful_name(s.name.as_deref())
        .or_else(|| meaningful_name(Some(&s.project)))
        .unwrap_or(s.tool.label())
        .to_owned();
    let tool_tag = match s.tool {
        Tool::Claude | Tool::Codex | Tool::Kimi | Tool::Pi => s.tool.label(),
        Tool::Generic => "",
    };
    let tool_color = s.tool.accent().rgb24();

    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .w_full()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(
                    div()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .truncate()
                        .text_color(rgb(palette.text_primary))
                        .text_size(px(13.0))
                        .child(label),
                )
                .when(!branch.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_none()
                            .text_color(rgb(palette.text_secondary))
                            .text_size(px(11.0))
                            .child(branch),
                    )
                }),
        )
        .when(!tool_tag.is_empty(), |d| {
            d.child(
                div()
                    .flex_none()
                    .text_color(rgb(tool_color))
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(tool_tag),
            )
        })
}

fn summary_cell(
    status: Status,
    summary: &str,
    id: SessionId,
    index: usize,
    cx: &mut Context<SessionsView>,
) -> AnyElement {
    let accent = status_color(status);
    let palette = current_palette();
    if status == Status::YourTurn {
        return div()
            .id(("ack", index))
            .flex_none()
            .px(px(7.0))
            .py(px(1.0))
            .rounded_md()
            .bg(rgba(palette.your_turn_badge_rgba))
            .text_color(rgb(accent))
            .text_size(px(11.0))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(palette.your_turn_hover_rgba)))
            .child(format!("{summary} \u{2713}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.acknowledge(&id);
                cx.stop_propagation();
                cx.notify();
            }))
            .into_any_element();
    }
    let indicator = div()
        .flex_none()
        .w(px(11.0))
        .text_color(rgb(accent))
        .text_size(px(10.0))
        .child(status_glyph(status));
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .min_w(px(0.0))
        .overflow_hidden()
        .child(indicator)
        .child(
            div()
                .min_w(px(0.0))
                .overflow_hidden()
                .truncate()
                .text_color(rgb(accent))
                .text_size(px(11.0))
                .child(summary.to_string()),
        )
        .into_any_element()
}

fn driving_chip(
    count: usize,
    driver: SessionId,
    index: usize,
    cx: &mut Context<SessionsView>,
) -> AnyElement {
    let palette = current_palette();
    div()
        .id(("cycle", index))
        .flex_none()
        .px(px(7.0))
        .py(px(1.0))
        .rounded_md()
        .bg(rgba(palette.bridged_badge_rgba))
        .text_color(rgb(palette.bridged))
        .text_size(px(11.0))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(palette.bridged_hover_rgba)))
        .child(format!("\u{21C4} {count}"))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.cycle_implementers_of(&driver, true, cx);
            cx.stop_propagation();
            cx.notify();
        }))
        .into_any_element()
}

fn status_line(s: &SessionState, index: usize, cx: &mut Context<SessionsView>) -> impl IntoElement {
    let palette = current_palette();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .w_full()
        .overflow_hidden()
        .child(summary_cell(s.status, &s.summary, s.id.clone(), index, cx))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.0))
                .when(s.bridged, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .text_color(rgb(palette.bridged))
                            .text_size(px(11.0))
                            .child("\u{21C4}"),
                    )
                })
                .when(!s.driving.is_empty(), |d| {
                    d.child(driving_chip(s.driving.len(), s.id.clone(), index, cx))
                })
                .child(
                    div()
                        .flex_none()
                        .text_color(rgb(palette.text_faint))
                        .text_size(px(11.0))
                        .child(meta_value(s)),
                ),
        )
}

fn session_row(
    s: &SessionState,
    selected: bool,
    index: usize,
    cx: &mut Context<SessionsView>,
) -> impl IntoElement {
    let tint = tint_color(s.status);
    let id = s.id.clone();
    let palette = current_palette();
    div()
        .id(("session-row", index))
        .w_full()
        .border_l_2()
        .border_color(if selected {
            rgb(palette.selection_border)
        } else {
            rgba(palette.transparent_rgba)
        })
        .cursor(CursorStyle::PointingHand)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.jump_to_session(id.clone(), "row-click", cx);
            cx.notify();
        }))
        .child(
            div()
                .relative()
                .w_full()
                .bg(rgba(tint))
                .border_b_1()
                .border_color(rgb(palette.divider))
                .child(
                    div()
                        .id(("row-hover", index))
                        .absolute()
                        .inset_0()
                        .hover(|style| style.bg(rgba(palette.keycap_bg_rgba))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .px(px(10.0))
                        .py(px(9.0))
                        .child(identity_line(s))
                        .child(status_line(s, index, cx)),
                ),
        )
}

enum StripAction {
    Expand,
    Dismiss,
}

fn strip_key_action(key: &str) -> Option<StripAction> {
    match key {
        "enter" | "space" | "up" => Some(StripAction::Expand),
        "escape" => Some(StripAction::Dismiss),
        _ => None,
    }
}

fn strip_click_activates(event: &ClickEvent, gesture: &DragGestureState) -> bool {
    let ClickEvent::Mouse(click) = event else {
        return false;
    };
    if gesture.is_moving() {
        return false;
    }
    let dx = click.up.position.x.to_f64() - click.down.position.x.to_f64();
    let dy = click.up.position.y.to_f64() - click.down.position.y.to_f64();
    (dx * dx + dy * dy).sqrt() < gesture.threshold()
}

impl SessionsView {
    fn render_strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.rows();
        let palette = current_palette();
        div()
            .id("cli-sessions-strip")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .rounded_lg()
            .border_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.chrome_bg))
            .font_family(SharedString::from("Menlo"))
            .cursor(CursorStyle::OpenHand)
            .panel_drag_after(&self.drag_gesture)
            .hover(|style| style.bg(rgba(palette.keycap_bg_rgba)))
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                if strip_click_activates(event, &this.drag_gesture.borrow()) {
                    this.expand_panel(window, cx);
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(
                |this, ev: &KeyDownEvent, window, cx| match strip_key_action(&ev.keystroke.key) {
                    Some(StripAction::Expand) => {
                        if this.key_repeat_guard(&ev.keystroke.key) {
                            this.expand_panel(window, cx);
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }
                    Some(StripAction::Dismiss) => {
                        this.dismiss_with_reason(STRIP_ESCAPE_REASON);
                    }
                    None => {}
                },
            ))
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _window, _cx| {
                this.key_released(&ev.keystroke.key);
            }))
            .child(summary_groups_el(&rows))
            .child(
                div()
                    .text_color(rgb(palette.text_heading))
                    .text_size(px(12.0))
                    .child("\u{25B2}"),
            )
            .into_any_element()
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.rows();
        let palette = current_palette();
        let order: Vec<SessionId> = rows.iter().map(|s| s.id.clone()).collect();
        let highlight = self.selection().highlight_index(&order);
        let is_empty = rows.is_empty();
        let row_els: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(i, s)| session_row(s, highlight == Some(i), i, cx))
            .collect();

        div()
            .id("cli-sessions")
            .track_focus(&self.focus_handle)
            .tab_stop(true)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.panel_bg))
            .font_family(SharedString::from("Menlo"))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "tab" => {
                        if ev.keystroke.modifiers.shift {
                            window.focus_prev();
                        } else {
                            window.focus_next();
                        }
                    }
                    "down" | "j" => {
                        this.move_selection_down();
                        cx.notify();
                    }
                    "up" | "k" => {
                        this.move_selection_up();
                        cx.notify();
                    }
                    "right" | "l" => {
                        this.cycle_implementers(true, cx);
                        cx.notify();
                    }
                    "left" | "h" => {
                        this.cycle_implementers(false, cx);
                        cx.notify();
                    }
                    "enter" => {
                        if this.key_repeat_guard("enter") {
                            this.focus_selected(cx);
                            cx.notify();
                        }
                    }
                    "a" => {
                        this.acknowledge_selected();
                        cx.notify();
                    }
                    "escape" => {
                        this.dismiss_with_reason(ESCAPE_REASON);
                    }
                    _ => {}
                }
            }))
            .child(header(&rows, cx))
            .child(
                div()
                    .id("cli-sessions-list")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .when(is_empty, |d| d.child(empty_state()))
                    .children(row_els),
            )
            .child(footer())
            .into_any_element()
    }
}

impl Render for SessionsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.is_collapsed() {
            self.render_strip(cx)
        } else {
            self.render_panel(cx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        point, px, Bounds, KeyboardButton, KeyboardClickEvent, Modifiers, MouseButton,
        MouseClickEvent, MouseDownEvent, MouseUpEvent,
    };

    fn mouse_down(x: f32, y: f32) -> MouseDownEvent {
        MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(x), px(y)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }
    }

    fn mouse_up(x: f32, y: f32) -> MouseUpEvent {
        MouseUpEvent {
            button: MouseButton::Left,
            position: point(px(x), px(y)),
            modifiers: Modifiers::default(),
            click_count: 1,
        }
    }

    fn mouse_click(down: (f32, f32), up: (f32, f32)) -> ClickEvent {
        ClickEvent::Mouse(MouseClickEvent {
            down: mouse_down(down.0, down.1),
            up: mouse_up(up.0, up.1),
        })
    }

    fn keyboard_click() -> ClickEvent {
        ClickEvent::Keyboard(KeyboardClickEvent {
            button: KeyboardButton::Enter,
            bounds: Bounds::new(point(px(0.0), px(0.0)), gpui::size(px(1.0), px(1.0))),
        })
    }

    fn gesture(moving: bool) -> DragGestureState {
        let mut g = DragGestureState::new(4.0);
        g.on_down(point(px(0.0), px(0.0)));
        if moving {
            g.on_move(point(px(10.0), px(0.0)), true);
        }
        g
    }

    #[test]
    fn activation_clicks_accept_mouse_and_reject_keyboard() {
        assert!(accepts_activation_click(&mouse_click(
            (1.0, 1.0),
            (1.0, 1.0)
        )));
        assert!(!accepts_activation_click(&keyboard_click()));
    }

    #[test]
    fn strip_clicks_expand_only_for_still_mouse_clicks() {
        assert!(strip_click_activates(
            &mouse_click((1.0, 1.0), (1.0, 1.0)),
            &gesture(false)
        ));
        assert!(!strip_click_activates(&keyboard_click(), &gesture(false)));
        assert!(!strip_click_activates(
            &mouse_click((1.0, 1.0), (1.0, 1.0)),
            &gesture(true)
        ));
        assert!(!strip_click_activates(
            &mouse_click((0.0, 0.0), (6.0, 0.0)),
            &gesture(false)
        ));
        assert!(strip_click_activates(
            &mouse_click((0.0, 0.0), (3.0, 0.0)),
            &gesture(false)
        ));
        assert!(!strip_click_activates(
            &mouse_click((0.0, 0.0), (4.0, 0.0)),
            &gesture(false)
        ));
    }

    #[test]
    fn a_gesture_that_crossed_the_threshold_never_expands_after_release() {
        let mut g = DragGestureState::new(4.0);
        g.on_down(point(px(0.0), px(0.0)));
        assert!(g.on_move(point(px(10.0), px(0.0)), true));
        g.on_up();
        assert!(!strip_click_activates(
            &mouse_click((0.0, 0.0), (10.0, 0.0)),
            &g
        ));
        g.on_down(point(px(0.0), px(0.0)));
        g.on_up();
        assert!(strip_click_activates(
            &mouse_click((0.0, 0.0), (0.0, 0.0)),
            &g
        ));
    }

    #[test]
    fn strip_key_mapping_keeps_the_panel_contract() {
        for key in ["enter", "space", "up"] {
            assert!(
                matches!(strip_key_action(key), Some(StripAction::Expand)),
                "{key}"
            );
        }
        assert!(matches!(
            strip_key_action("escape"),
            Some(StripAction::Dismiss)
        ));
        for key in ["tab", "down", "a", "x"] {
            assert!(strip_key_action(key).is_none(), "{key}");
        }
    }

    #[test]
    fn dismiss_reasons_are_distinct_per_surface() {
        assert_eq!(HIDE_BUTTON_REASON, "hide-button");
        assert_eq!(ESCAPE_REASON, "escape");
        assert_eq!(STRIP_ESCAPE_REASON, "strip-escape");
        assert_ne!(ESCAPE_REASON, STRIP_ESCAPE_REASON);
    }
}
