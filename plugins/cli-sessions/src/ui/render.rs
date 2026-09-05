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
    status.definition().idle
}

fn status_dot_el(
    kit: &qol_gpui::kit::Kit,
    status: Status,
    id: impl Into<gpui::ElementId>,
) -> AnyElement {
    let (tone, halo) = (status.definition().colors)(&current_palette());
    kit.animated_status_dot(id, tone, halo, status.is_active())
}

fn live_count(rows: &[SessionState]) -> usize {
    rows.iter().filter(|row| !is_idle(row.status)).count()
}

fn header(
    rows: &[SessionState],
    view: &SessionsView,
    cx: &mut Context<SessionsView>,
) -> impl IntoElement {
    let collapsed = view.is_collapsed();
    let palette = current_palette();
    let kit = qol_gpui::kit::kit();
    div()
        .flex_none()
        .h(px(collapse::STRIP_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .px(px(qol_gpui::theme::SPACE_PAD))
        .bg(rgb(palette.band_bg))
        .border_b(px(1.0))
        .border_color(rgba(kit.washes.hairline.packed()))
        .child(
            div()
                .id("sessions-header-title")
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .cursor(CursorStyle::OpenHand)
                .when(!collapsed, |title| title.panel_drag_area())
                .when(collapsed, |title| {
                    title
                        .panel_drag_after(&view.drag_gesture)
                        .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                            if strip_click_activates(event, &this.drag_gesture.borrow()) {
                                this.expand_panel(window, cx);
                                cx.notify();
                            }
                        }))
                })
                .text_color(rgb(palette.text_heading))
                .text_size(px(qol_gpui::theme::TEXT_BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Sessions"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(qol_gpui::theme::SPACE_STACK))
                .child(kit.count_chip_small(live_count(rows), "live"))
                .child(panel_controls(collapsed, cx)),
        )
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

fn panel_controls(collapsed: bool, cx: &mut Context<SessionsView>) -> impl IntoElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(qol_gpui::theme::SPACE_STACK))
        .child(header_control(
            "toggle-panel-button",
            if collapsed {
                qol_gpui::kit::WindowControlIcon::Expand
            } else {
                qol_gpui::kit::WindowControlIcon::Collapse
            },
            |this, window, cx| {
                if this.is_collapsed() {
                    this.expand_panel(window, cx);
                } else {
                    this.collapse_panel(window, cx);
                }
                cx.notify();
            },
            cx,
        ))
        .child(header_control(
            "hide-panel-button",
            qol_gpui::kit::WindowControlIcon::Close,
            |this, _, _| {
                this.dismiss_with_reason("hide-button");
            },
            cx,
        ))
}

fn header_control(
    id: &'static str,
    icon: qol_gpui::kit::WindowControlIcon,
    activate: fn(&mut SessionsView, &mut Window, &mut Context<SessionsView>),
    cx: &mut Context<SessionsView>,
) -> impl IntoElement {
    let kit = qol_gpui::kit::kit();
    kit.window_control(icon)
        .id(id)
        .focusable()
        .tab_stop(true)
        .cursor(CursorStyle::PointingHand)
        .focus(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            if matches!(event, ClickEvent::Mouse(_)) {
                activate(this, window, cx);
                cx.stop_propagation();
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                if this.key_repeat_guard(&event.keystroke.key) {
                    activate(this, window, cx);
                }
                cx.stop_propagation();
            }
        }))
}

fn footer() -> impl IntoElement {
    let kit = qol_gpui::kit::kit();
    kit.hint_bar_compact()
        .justify_center()
        .child(kit.hint_label_first("Focus", "Enter"))
        .child(kit.hint_label_first("Acknowledge", "A"))
        .child(kit.hint_label_first("Collapse", chord("alt+s")))
}

fn session_summary(s: &SessionState, cx: &mut Context<SessionsView>) -> AnyElement {
    let kit = qol_gpui::kit::kit();
    if s.status == Status::YourTurn {
        let id = s.id.clone();
        let (tone, _) = (s.status.definition().colors)(&current_palette());
        return div()
            .flex()
            .min_w_0()
            .h(px(qol_gpui::theme::SPACE_PAD))
            .child(
                kit.status_pill("your turn ✓", tone)
                    .id(SharedString::from(format!("ack-{}", s.id)))
                    .cursor(CursorStyle::PointingHand)
                    .hover(|style| style.bg(rgba(current_palette().your_turn_hover_rgba)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.acknowledge(&id);
                        cx.notify();
                        cx.stop_propagation();
                    })),
            )
            .into_any_element();
    }
    text_line(
        if s.bridged && s.driving.is_empty() {
            format!("{} · delegated", s.summary)
        } else {
            s.summary.clone()
        },
        qol_gpui::theme::TEXT_NANO,
        FontWeight::NORMAL,
        kit.palette.text_muted,
    )
    .h(px(qol_gpui::theme::SPACE_PAD))
    .into_any_element()
}

fn text_line(text: String, size: f32, weight: FontWeight, color: u32) -> gpui::Div {
    div().flex().w_full().min_w_0().overflow_hidden().child(
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

fn agent_chip(
    session: &SessionState,
    index: usize,
    cx: &mut Context<SessionsView>,
) -> impl IntoElement {
    let kit = qol_gpui::kit::kit();
    let driver = session.id.clone();
    kit.count_button(session.driving.len())
        .id(("cycle-agents", index))
        .cursor(CursorStyle::PointingHand)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.cycle_implementers_of(&driver, true, cx);
            cx.stop_propagation();
        }))
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
    let (tone, _) = (s.status.definition().colors)(&current_palette());
    let tint = qol_gpui::theme::tinted_row_palette(tone, kit.palette);
    let name = meaningful_name(s.name.as_deref())
        .or_else(|| meaningful_name(Some(&s.project)))
        .unwrap_or(&s.tool.label)
        .to_owned();
    let row = kit
        .row_compact_described()
        .id(SharedString::from(format!("session-row-{}", s.id)))
        .group("session-row")
        .flex_none()
        .pl(px(
            qol_gpui::vertical_label::WIDTH + qol_gpui::theme::SPACE_INSET
        ))
        .pr(px(qol_gpui::theme::SPACE_PAD))
        .overflow_hidden()
        .gap(px(qol_gpui::theme::SPACE_CELL))
        .cursor(CursorStyle::PointingHand)
        .hover(move |style| {
            style.bg(rgba(
                if selected { tint.selected } else { tint.hover }.packed(),
            ))
        })
        .child(kit.vertical_identity_tab(s.tool.label.clone(), s.tool.accent.rgb24()))
        .child(status_dot_el(
            &kit,
            s.status,
            SharedString::from(format!("session-status-{}", s.id)),
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(qol_gpui::theme::SPACE_STACK))
                .child(
                    text_line(
                        name,
                        qol_gpui::theme::TEXT_CAPTION,
                        FontWeight::SEMIBOLD,
                        if idle {
                            kit.palette.text_secondary
                        } else {
                            kit.palette.text_primary
                        },
                    )
                    .h(px(qol_gpui::theme::SPACE_PAD)),
                )
                .child(session_summary(s, cx)),
        )
        .child(
            div()
                .flex_none()
                .w(px(qol_gpui::theme::HEIGHT_INLINE))
                .when(!s.driving.is_empty(), |slot| {
                    slot.child(agent_chip(s, index, cx))
                }),
        )
        .child(
            kit.row_metadata()
                .child(div().h(px(qol_gpui::theme::SPACE_PAD)))
                .child(
                    div()
                        .flex_none()
                        .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                        .text_size(px(qol_gpui::theme::TEXT_NANO))
                        .text_color(rgb(kit.palette.text_muted))
                        .child(format_elapsed(s.last_activity)),
                ),
        )
        .when(!selected, |row| {
            row.child(
                kit.row_separator()
                    .group_hover("session-row", |style| style.opacity(0.0)),
            )
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.jump_to_session(id.clone(), "row-click", cx);
            cx.notify();
        }));
    kit.row_selected_tinted_after(row, selected, tone, qol_gpui::vertical_label::WIDTH)
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

impl SessionsView {
    fn render_panel(
        &self,
        rows: &[SessionState],
        body_visible: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = current_palette();
        let order: Vec<SessionId> = rows.iter().map(|s| s.id.clone()).collect();
        let highlight = self.selection().highlight_index(&order);
        let is_empty = rows.is_empty();
        if body_visible {
            self.list_scroll.follow(highlight);
        }
        let row_els: Vec<_> = rows
            .iter()
            .filter(|_| body_visible)
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
            .rounded_none()
            .overflow_hidden()
            .bg(rgb(palette.panel_bg))
            .shadow(panel_shadow(&palette))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if this.is_collapsed() && ev.keystroke.key != "tab" {
                    match strip_key_action(&ev.keystroke.key, &ev.keystroke.modifiers) {
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
                    }
                    return;
                }
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
                        if ev.keystroke.modifiers == Modifiers::default()
                            && this.key_repeat_guard("a")
                        {
                            this.acknowledge_selected();
                            cx.notify();
                        }
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
            .child(header(rows, self, cx))
            .when(body_visible, |panel| {
                panel
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
                            .when(is_empty, |d| d.child(empty_state()))
                            .children(row_els),
                    )
                    .child(footer())
            })
            .into_any_element()
    }
}

impl Render for SessionsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows();
        let body_visible = window.viewport_size().height > px(collapse::STRIP_HEIGHT);
        let active = body_visible && rows.iter().any(|row| row.status.is_active());
        let content = self.render_panel(&rows, body_visible, cx);
        qol_gpui::activity_animation::ActivityAnimation::new(
            "sessions-activity-animation",
            self.is_showing() && active,
            content,
        )
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
            runtime_status: None,
        }
    }

    #[test]
    fn header_live_count_excludes_inactive_sessions() {
        let rows = vec![
            state(Status::Working),
            state(Status::NeedsYou),
            state(Status::YourTurn),
            state(Status::Unknown),
            state(Status::Acknowledged),
        ];
        assert_eq!(live_count(&rows), 3);
        assert_eq!(live_count(&[]), 0);
    }
}
