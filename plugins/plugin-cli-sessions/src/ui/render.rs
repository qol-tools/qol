use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, Context, FontWeight, KeyDownEvent, SharedString, Window,
};

use crate::registry::SessionState;
use crate::status::Status;
use crate::ui::SessionsView;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn format_elapsed(last_activity: u64) -> String {
    let secs = now_secs().saturating_sub(last_activity);
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
    match status {
        Status::NeedsYou => 0xf85149,
        Status::YourTurn => 0xd29922,
        Status::Working => 0x3fb950,
        Status::Unknown => 0x6e7681,
        Status::Acknowledged => 0x6e7681,
    }
}

fn tint_color(status: Status) -> u32 {
    match status {
        Status::NeedsYou => 0xf8514922,
        Status::YourTurn => 0xd2992222,
        Status::Working => 0x3fb9501e,
        Status::Unknown => 0x00000000,
        Status::Acknowledged => 0x00000000,
    }
}

fn header(count: usize) -> impl IntoElement {
    div()
        .h(px(34.0))
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .px(px(12.0))
        .bg(rgb(0x0d1117u32))
        .border_b_1()
        .border_color(rgb(0x21262du32))
        .child(
            div()
                .text_color(rgb(0xc9d1d9u32))
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child("CLI SESSIONS"),
        )
        .child(
            div()
                .text_color(rgb(0x6e7681u32))
                .text_size(px(11.0))
                .child(format!("{count}")),
        )
}

fn empty_state() -> impl IntoElement {
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
                .text_color(rgb(0xc9d1d9u32))
                .text_size(px(13.0))
                .child("No CLI sessions found"),
        )
        .child(
            div()
                .text_color(rgb(0x7d8590u32))
                .text_size(px(11.0))
                .child("Open a CLI in kitty, then open this panel again."),
        )
}

fn summary_cell(
    status: Status,
    summary: &str,
    window_id: u64,
    index: usize,
    cx: &mut Context<SessionsView>,
) -> AnyElement {
    let accent = status_color(status);
    if status == Status::YourTurn {
        return div()
            .id(("ack", index))
            .flex_none()
            .px(px(7.0))
            .py(px(1.0))
            .rounded_md()
            .bg(rgba(0xd2992233))
            .text_color(rgb(accent))
            .text_size(px(11.0))
            .cursor_pointer()
            .hover(|s| s.bg(rgba(0xd2992255)))
            .child(format!("{summary} \u{2713}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.acknowledge(window_id);
                cx.stop_propagation();
                cx.notify();
            }))
            .into_any_element();
    }
    div()
        .min_w(px(0.0))
        .overflow_hidden()
        .truncate()
        .text_color(rgb(accent))
        .text_size(px(11.0))
        .child(summary.to_string())
        .into_any_element()
}

fn session_row(
    s: &SessionState,
    selected: bool,
    index: usize,
    cx: &mut Context<SessionsView>,
) -> impl IntoElement {
    let branch = s.branch.clone().unwrap_or_default();
    let label = s.name.clone().unwrap_or_else(|| s.project.clone());
    let tint = tint_color(s.status);
    let window_id = s.window_id;
    let (tool_tag, tool_color) = match s.tool {
        crate::tool::Tool::Claude => ("Claude", 0xd97757u32),
        crate::tool::Tool::Codex => ("Codex", 0x10a37fu32),
        crate::tool::Tool::Generic => ("", 0x6e7681u32),
    };

    div()
        .id(("session-row", index))
        .w_full()
        .border_l_2()
        .border_color(if selected {
            rgb(0x58a6ffu32)
        } else {
            rgba(0x00000000u32)
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.jump_to(index, "row-click", cx);
            cx.notify();
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .w_full()
                .px(px(11.0))
                .py(px(9.0))
                .bg(rgba(tint))
                .border_b_1()
                .border_color(rgb(0x21262du32))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .w_full()
                        .overflow_hidden()
                        .child(
                            div()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .truncate()
                                .text_color(rgb(0xe6edf3u32))
                                .text_size(px(13.0))
                                .child(label),
                        )
                        .when(!branch.is_empty(), |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .text_color(rgb(0x8b949eu32))
                                    .text_size(px(11.0))
                                    .child(branch),
                            )
                        })
                        .when(!tool_tag.is_empty(), |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .text_color(rgb(tool_color))
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(tool_tag),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .w_full()
                        .overflow_hidden()
                        .child(summary_cell(s.status, &s.summary, window_id, index, cx))
                        .child(
                            div()
                                .flex_none()
                                .text_color(rgb(0x6e7681u32))
                                .text_size(px(11.0))
                                .child(format_elapsed(s.last_activity)),
                        ),
                ),
        )
}

impl Render for SessionsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows();
        if rows.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(rows.len() - 1);
        }
        let selected = self.selected;
        let is_empty = rows.is_empty();
        let count = rows.len();
        let row_els: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(i, s)| session_row(s, i == selected, i, cx))
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
            .border_color(rgb(0x30363du32))
            .bg(rgb(0x161b22u32))
            .font_family(SharedString::from("Menlo"))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| {
                let len = this.rows().len();
                match ev.keystroke.key.as_str() {
                    "down" | "j" => {
                        this.selected = (this.selected + 1).min(len.saturating_sub(1));
                        cx.notify();
                    }
                    "up" | "k" => {
                        this.selected = this.selected.saturating_sub(1);
                        cx.notify();
                    }
                    "enter" => {
                        let i = this.selected;
                        this.jump_to(i, "enter", cx);
                        cx.notify();
                    }
                    "a" => {
                        this.acknowledge_selected();
                        cx.notify();
                    }
                    "escape" => {
                        this.dismiss_with_reason("escape");
                    }
                    d if d.len() == 1 && matches!(d.chars().next(), Some('1'..='9')) => {
                        let idx = d.parse::<usize>().unwrap_or(0).saturating_sub(1);
                        if idx < len {
                            this.jump_to(idx, "number", cx);
                            cx.notify();
                        }
                    }
                    _ => {}
                }
            }))
            .child(header(count))
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
    }
}
