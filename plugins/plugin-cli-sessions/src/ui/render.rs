use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, Animation, AnimationExt, AnyElement, Context, FontWeight, KeyDownEvent,
    SharedString, Window,
};

use crate::registry::SessionState;
use crate::status::Status;
use crate::tool::Tool;
use crate::ui::SessionsView;

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
    match status {
        Status::NeedsYou => 0xf85149,
        Status::YourTurn => 0xd29922,
        Status::Working => 0x3fb950,
        Status::Service => 0x58a6ff,
        Status::Unknown => 0x6e7681,
        Status::Acknowledged => 0x6e7681,
    }
}

fn tint_color(status: Status) -> u32 {
    match status {
        Status::NeedsYou => 0xf8514922,
        Status::YourTurn => 0xd2992222,
        Status::Working => 0x3fb9501e,
        Status::Service => 0x58a6ff14,
        Status::Unknown => 0x00000000,
        Status::Acknowledged => 0x00000000,
    }
}

const SPINNER_FRAMES: [&str; 10] = [
    "\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280F}",
];

fn status_glyph(status: Status) -> &'static str {
    match status {
        Status::NeedsYou => "!",
        Status::Working => SPINNER_FRAMES[0],
        Status::Service => "\u{25CF}",
        Status::Unknown | Status::Acknowledged => "\u{00B7}",
        Status::YourTurn => "",
    }
}

fn spinner(index: usize, color: u32) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(11.0))
        .text_color(rgb(color))
        .text_size(px(10.0))
        .with_animation(
            ("spinner", index),
            Animation::new(Duration::from_millis(800)).repeat(),
            |el, delta| {
                let i =
                    ((delta * SPINNER_FRAMES.len() as f32) as usize).min(SPINNER_FRAMES.len() - 1);
                el.child(SPINNER_FRAMES[i])
            },
        )
}

fn pulse(index: usize, color: u32) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(11.0))
        .flex()
        .items_center()
        .justify_center()
        .child(div().w(px(7.0)).h(px(7.0)).rounded_full().with_animation(
            ("pulse", index),
            Animation::new(Duration::from_millis(1800)).repeat(),
            move |el, delta| {
                let wave = (delta * std::f32::consts::TAU).sin() * 0.5 + 0.5;
                let alpha = (0.4 + 0.6 * wave).clamp(0.0, 1.0);
                let a = (alpha * 255.0) as u32;
                el.bg(rgba((color << 8) | a))
            },
        ))
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
    let colors = [0xf85149u32, 0xd29922, 0x3fb950, 0x58a6ff, 0x6e7681];
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
        .child(div().flex().items_center().gap(px(9.0)).children(
            summary_groups(rows).into_iter().map(|(color, count)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(rgb(color)))
                    .child(
                        div()
                            .text_color(rgb(0x8b949eu32))
                            .text_size(px(11.0))
                            .child(format!("{count}")),
                    )
            }),
        ))
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

fn key_hint(key: &'static str, label: &'static str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .text_color(rgb(0x6e7681u32))
        .text_size(px(10.0))
        .child(
            div()
                .text_color(rgb(0xc9d1d9u32))
                .text_size(px(9.0))
                .bg(rgba(0xffffff0fu32))
                .border_1()
                .border_color(rgb(0x30363du32))
                .rounded(px(4.0))
                .px(px(5.0))
                .py(px(1.0))
                .child(key),
        )
        .child(label)
}

fn footer() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .w_full()
        .px(px(11.0))
        .py(px(7.0))
        .bg(rgb(0x0d1117u32))
        .border_t_1()
        .border_color(rgb(0x21262du32))
        .child(key_hint("\u{2191}\u{2193}", "move"))
        .child(key_hint("\u{23CE}", "jump"))
        .child(key_hint("a", "ack"))
        .child(key_hint("esc", "close"))
}

fn gutter(index: usize, selected: bool) -> impl IntoElement {
    let num = if index < 9 {
        (index + 1).to_string()
    } else {
        String::new()
    };
    div()
        .flex_none()
        .w(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .border_r_1()
        .border_color(rgb(0x21262du32))
        .text_size(px(11.0))
        .text_color(rgb(if selected { 0x58a6ffu32 } else { 0x6e7681u32 }))
        .child(num)
}

fn identity_line(s: &SessionState) -> impl IntoElement {
    let branch = s.branch.clone().unwrap_or_default();
    let label = s.name.clone().unwrap_or_else(|| s.project.clone());
    let (tool_tag, tool_color) = match s.tool {
        Tool::Claude => ("Claude", 0xd97757u32),
        Tool::Codex => ("Codex", 0x10a37fu32),
        Tool::Generic => ("", 0x6e7681u32),
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
    let indicator: AnyElement = if status == Status::Working {
        spinner(index, accent).into_any_element()
    } else if status == Status::Service {
        pulse(index, accent).into_any_element()
    } else {
        div()
            .flex_none()
            .w(px(11.0))
            .text_color(rgb(accent))
            .text_size(px(10.0))
            .child(status_glyph(status))
            .into_any_element()
    };
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
                .text_color(rgb(0x6e7681u32))
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
                .w_full()
                .bg(rgba(tint))
                .border_b_1()
                .border_color(rgb(0x21262du32))
                .child(gutter(index, selected))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
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
