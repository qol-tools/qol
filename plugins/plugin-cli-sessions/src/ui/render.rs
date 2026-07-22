use std::sync::LazyLock;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, Context, CursorStyle, FontWeight, KeyDownEvent, MouseButton,
    SharedString, Window,
};
use qol_gpui::theme::{cli_sessions_runtime, CliSessionsPalette};

use crate::session::registry::SessionState;
use crate::session::status::Status;
use crate::session::tool::Tool;
use crate::ui::SessionsView;

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
    let since = if matches!(s.status, Status::Working | Status::Service) {
        s.running_since.unwrap_or(s.last_activity)
    } else {
        s.last_activity
    };
    format_elapsed(since)
}

fn header(rows: &[SessionState]) -> impl IntoElement {
    let palette = current_palette();
    div()
        .h(px(34.0))
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .px(px(12.0))
        .bg(rgb(palette.chrome_bg))
        .border_b_1()
        .border_color(rgb(palette.divider))
        .cursor(CursorStyle::OpenHand)
        .on_mouse_down(MouseButton::Left, |_event, window, _cx| {
            window.start_window_move();
        })
        .child(
            div()
                .text_color(rgb(palette.text_heading))
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child("CLI SESSIONS"),
        )
        .child(div().flex().items_center().gap(px(9.0)).children(
            summary_groups(rows).into_iter().map(|(color, count)| {
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
            }),
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
        .child(key_hint("\u{23CE}", "jump"))
        .child(key_hint("a", "ack"))
        .child(key_hint("esc", "close"))
}

fn identity_line(s: &SessionState) -> impl IntoElement {
    let palette = current_palette();
    let branch = s.branch.clone().unwrap_or_default();
    let label = s.name.clone().unwrap_or_else(|| s.project.clone());
    let (tool_tag, tool_color) = match s.tool {
        Tool::Claude => ("Claude", palette.claude),
        Tool::Codex => ("Codex", palette.codex),
        Tool::Generic => ("", palette.text_faint),
    };

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
    window_id: u64,
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
                this.acknowledge(window_id);
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

fn status_line(s: &SessionState, index: usize, cx: &mut Context<SessionsView>) -> impl IntoElement {
    let palette = current_palette();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .w_full()
        .overflow_hidden()
        .child(summary_cell(s.status, &s.summary, s.window_id, index, cx))
        .child(
            div()
                .flex_none()
                .text_color(rgb(palette.text_faint))
                .text_size(px(11.0))
                .child(meta_value(s)),
        )
}

fn session_row(
    s: &SessionState,
    selected: bool,
    index: usize,
    cx: &mut Context<SessionsView>,
) -> impl IntoElement {
    let tint = tint_color(s.status);
    let window_id = s.window_id;
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
        .on_click(cx.listener(move |this, _, _, cx| {
            this.jump_to_window(window_id, "row-click", cx);
            cx.notify();
        }))
        .child(
            div()
                .w_full()
                .bg(rgba(tint))
                .border_b_1()
                .border_color(rgb(palette.divider))
                .flex()
                .flex_col()
                .gap(px(3.0))
                .px(px(10.0))
                .py(px(9.0))
                .child(identity_line(s))
                .child(status_line(s, index, cx)),
        )
}

impl Render for SessionsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows();
        let palette = current_palette();
        let order: Vec<u64> = rows.iter().map(|s| s.window_id).collect();
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
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.panel_bg))
            .font_family(SharedString::from("Menlo"))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| {
                match ev.keystroke.key.as_str() {
                    "down" | "j" => {
                        this.move_selection_down();
                        cx.notify();
                    }
                    "up" | "k" => {
                        this.move_selection_up();
                        cx.notify();
                    }
                    "enter" => {
                        this.focus_selected(cx);
                        cx.notify();
                    }
                    "a" => {
                        this.acknowledge_selected();
                        cx.notify();
                    }
                    "escape" => {
                        this.dismiss_with_reason("escape");
                    }
                    _ => {}
                }
            }))
            .child(header(&rows))
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
    }
}
