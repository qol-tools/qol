use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, ClickEvent, Context, CursorStyle, FontWeight, KeyDownEvent,
    KeyUpEvent, Modifiers, SharedString, Window,
};
use qol_gpui::surface::{DragGestureState, PanelDragArea};
use qol_gpui::theme::{cli_sessions_runtime, CliSessionsPalette};
use qol_terminal_sessions::SessionId;

use crate::session::registry::{meaningful_name, SessionState};
use crate::session::status::Status;
use crate::ui::collapse;
use crate::ui::SessionsView;

const CLOSE_KEY_REASON: &str = "close-key";
const ESCAPE_REASON: &str = "escape";
const STRIP_ESCAPE_REASON: &str = "strip-escape";

fn current_palette() -> CliSessionsPalette {
    cli_sessions_runtime()
}

fn panel_shadow(palette: &CliSessionsPalette) -> Vec<gpui::BoxShadow> {
    qol_gpui::kit::float_shadow(palette.text_primary)
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

fn is_idle(status: Status) -> bool {
    matches!(status, Status::Unknown | Status::Acknowledged)
}

fn status_dot_el(kit: &qol_gpui::kit::Kit, status: Status) -> gpui::Div {
    let (tone, halo) = match status {
        Status::NeedsYou | Status::YourTurn => {
            (kit.palette.warning, kit.washes.halo_attention.packed())
        }
        Status::Working | Status::Service => {
            (kit.palette.success, kit.washes.halo_success.packed())
        }
        Status::Unknown | Status::Acknowledged => {
            (kit.palette.text_muted, kit.washes.fill_resting.packed())
        }
    };
    kit.status_dot(tone, halo)
}

fn live_count(rows: &[SessionState]) -> usize {
    rows.iter().filter(|row| !is_idle(row.status)).count()
}

fn waiting_count(rows: &[SessionState]) -> usize {
    rows.iter()
        .filter(|row| matches!(row.status, Status::NeedsYou | Status::YourTurn))
        .count()
}

fn worst_status(rows: &[SessionState]) -> Status {
    rows.iter()
        .map(|row| row.status)
        .min_by_key(|status| match status {
            Status::NeedsYou => 0,
            Status::YourTurn => 1,
            Status::Working => 2,
            Status::Service => 3,
            Status::Acknowledged => 4,
            Status::Unknown => 5,
        })
        .unwrap_or(Status::Unknown)
}

fn header(rows: &[SessionState], _cx: &mut Context<SessionsView>) -> impl IntoElement {
    let palette = current_palette();
    let kit = qol_gpui::kit::kit();
    div()
        .flex_none()
        .h(px(qol_gpui::theme::HEIGHT_SETTING_ROW))
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .px(px(qol_gpui::theme::SPACE_PAD))
        .bg(rgb(palette.band_bg))
        .border_b(px(1.0))
        .border_color(rgba(kit.washes.hairline.packed()))
        .cursor(CursorStyle::OpenHand)
        .panel_drag_area()
        .child(
            div()
                .text_color(rgb(palette.text_heading))
                .text_size(px(qol_gpui::theme::TEXT_BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Sessions"),
        )
        .child(kit.count_chip(live_count(rows), "live"))
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
                .text_size(px(qol_gpui::theme::TEXT_BODY))
                .child("No sessions running"),
        )
        .child(
            div()
                .text_color(rgb(palette.text_muted))
                .text_size(px(qol_gpui::theme::TEXT_NANO))
                .child("spawned lanes appear here"),
        )
}

fn chord(input: &str) -> String {
    qol_hotkeys::chord::label_for(input).unwrap_or_default()
}

fn footer() -> impl IntoElement {
    let kit = qol_gpui::kit::kit();
    kit.hint_bar()
        .child(kit.hint(chord("enter"), "focus"))
        .child(kit.hint(chord("platform+w"), "close"))
        .child(div().flex_1())
        .child(kit.hint(chord("alt+s"), "collapse"))
}

fn subtitle_text(s: &SessionState) -> String {
    let elapsed = format_elapsed(s.last_activity);
    let summary = s.summary.trim();
    if summary.is_empty() {
        elapsed
    } else {
        format!("{summary} \u{00B7} {elapsed}")
    }
}

fn text_line(text: String, size: f32, weight: FontWeight, color: u32) -> gpui::Div {
    div().flex().w_full().overflow_hidden().child(
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_size(px(size))
            .font_weight(weight)
            .text_color(rgb(color))
            .child(text),
    )
}

fn session_row(
    s: &SessionState,
    selected: bool,
    index: usize,
    cx: &mut Context<SessionsView>,
) -> impl IntoElement {
    let id = s.id.clone();
    let kit = qol_gpui::kit::kit();
    let idle = is_idle(s.status);
    let name = meaningful_name(s.name.as_deref())
        .or_else(|| meaningful_name(Some(&s.project)))
        .unwrap_or(&s.tool.label)
        .to_owned();
    let row = div()
        .id(("session-row", index))
        .flex_none()
        .w_full()
        .h(px(qol_gpui::theme::HEIGHT_RULE_ROW))
        .px(px(qol_gpui::theme::SPACE_PAD))
        .overflow_hidden()
        .flex()
        .items_center()
        .gap(px(12.0))
        .rounded(px(qol_gpui::theme::RADIUS_CONTROL))
        .cursor(CursorStyle::PointingHand)
        .hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
        .child(status_dot_el(&kit, s.status))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(text_line(
                    name,
                    qol_gpui::theme::TEXT_CAPTION,
                    if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    },
                    if idle {
                        kit.palette.text_secondary
                    } else {
                        kit.palette.text_primary
                    },
                ))
                .child(text_line(
                    subtitle_text(s),
                    if idle {
                        qol_gpui::theme::TEXT_NANO
                    } else {
                        qol_gpui::theme::TEXT_MICRO
                    },
                    FontWeight::NORMAL,
                    if idle {
                        kit.palette.text_muted
                    } else {
                        kit.palette.text_secondary
                    },
                )),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.jump_to_session(id.clone(), "row-click", cx);
            cx.notify();
        }));
    kit.row_selected(row, selected)
}

enum StripAction {
    Expand,
    Dismiss,
}

fn strip_key_action(key: &str, modifiers: &Modifiers) -> Option<StripAction> {
    match key {
        "s" if modifiers.alt => Some(StripAction::Expand),
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

fn strip_label(rows: &[SessionState]) -> String {
    let live = live_count(rows);
    let waiting = waiting_count(rows);
    if waiting == 0 {
        format!("{live} live")
    } else {
        format!("{live} live \u{00B7} {waiting} waiting")
    }
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
            .px(px(13.0))
            .gap(px(10.0))
            .rounded(px(qol_gpui::theme::RADIUS_WINDOW))
            .overflow_hidden()
            .bg(rgb(palette.panel_bg))
            .shadow(panel_shadow(&palette))
            .cursor(CursorStyle::OpenHand)
            .panel_drag_after(&self.drag_gesture)
            .hover(move |style| {
                style.bg(rgb(collapse::strip_hover_bg(
                    palette.panel_bg,
                    palette.keycap_bg_rgba,
                )))
            })
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                if strip_click_activates(event, &this.drag_gesture.borrow()) {
                    this.expand_panel(window, cx);
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(
                |this, ev: &KeyDownEvent, window, cx| match strip_key_action(
                    &ev.keystroke.key,
                    &ev.keystroke.modifiers,
                ) {
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
            .child(status_dot_el(&qol_gpui::kit::kit(), worst_status(&rows)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_color(rgb(palette.text_secondary))
                    .text_size(px(qol_gpui::theme::TEXT_MICRO))
                    .child(strip_label(&rows)),
            )
            .child(
                div()
                    .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                    .text_color(rgb(palette.text_muted))
                    .text_size(px(qol_gpui::theme::TEXT_NANO))
                    .child(chord("alt+s")),
            )
            .into_any_element()
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.rows();
        let palette = current_palette();
        let order: Vec<SessionId> = rows.iter().map(|s| s.id.clone()).collect();
        let highlight = self.selection().highlight_index(&order);
        let is_empty = rows.is_empty();
        self.list_scroll.follow(highlight);
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
            .rounded(px(qol_gpui::theme::RADIUS_WINDOW))
            .overflow_hidden()
            .bg(rgb(palette.panel_bg))
            .shadow(panel_shadow(&palette))
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
                    "w" if ev.keystroke.modifiers.platform => {
                        this.dismiss_with_reason(CLOSE_KEY_REASON);
                    }
                    "s" if ev.keystroke.modifiers.alt => {
                        this.collapse_panel(window, cx);
                        cx.notify();
                    }
                    "escape" => {
                        this.dismiss_with_reason(ESCAPE_REASON);
                    }
                    _ => {}
                }
            }))
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _window, _cx| {
                this.key_released(&ev.keystroke.key);
            }))
            .child(header(&rows, cx))
            .child(
                div()
                    .id("cli-sessions-list")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .track_scroll(self.list_scroll.handle())
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .py(px(6.0))
                    .px(px(8.0))
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
        let plain = Modifiers::default();
        for key in ["enter", "space", "up"] {
            assert!(
                matches!(strip_key_action(key, &plain), Some(StripAction::Expand)),
                "{key}"
            );
        }
        assert!(matches!(
            strip_key_action("escape", &plain),
            Some(StripAction::Dismiss)
        ));
        for key in ["tab", "down", "a", "x"] {
            assert!(strip_key_action(key, &plain).is_none(), "{key}");
        }
    }

    #[test]
    fn the_strip_answers_the_collapse_chord_it_advertises() {
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        assert!(matches!(
            strip_key_action("s", &alt),
            Some(StripAction::Expand)
        ));
        assert!(strip_key_action("s", &Modifiers::default()).is_none());
    }

    #[test]
    fn dismiss_reasons_are_distinct_per_surface() {
        assert_eq!(CLOSE_KEY_REASON, "close-key");
        assert_eq!(ESCAPE_REASON, "escape");
        assert_eq!(STRIP_ESCAPE_REASON, "strip-escape");
        assert_ne!(ESCAPE_REASON, STRIP_ESCAPE_REASON);
    }

    fn state(status: Status) -> SessionState {
        SessionState {
            id: SessionId::new(qol_terminal_sessions::kitty::backend_id().clone(), "k1_1.3")
                .unwrap(),
            root_pid: 1,
            project: "proj".into(),
            name: None,
            cwd: "/a/b/proj".into(),
            branch: None,
            tool: qol_terminal_sessions::cli::generic_tool(),
            status,
            summary: "x".into(),
            last_activity: 0,
            screen_hash: None,
            working_since: None,
            settled_since: None,
            bridged: false,
            driving: Vec::new(),
        }
    }

    #[test]
    fn the_strip_label_counts_live_and_waiting() {
        let rows = vec![
            state(Status::Working),
            state(Status::NeedsYou),
            state(Status::YourTurn),
            state(Status::Unknown),
        ];
        assert_eq!(strip_label(&rows), "3 live \u{00B7} 2 waiting");
        assert_eq!(strip_label(&rows[..1]), "1 live");
    }

    #[test]
    fn the_strip_dot_shows_the_most_urgent_status() {
        let rows = vec![state(Status::Working), state(Status::NeedsYou)];
        assert_eq!(worst_status(&rows), Status::NeedsYou);
        assert_eq!(worst_status(&[]), Status::Unknown);
    }
}
