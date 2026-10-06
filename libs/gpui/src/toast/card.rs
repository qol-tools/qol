use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::*;
use qol_theme::TextStyle;

use super::{RowId, SlabSnapshotRow, ToastTone};
use crate::kit::Kit;
use crate::text::{cased, TextStyled};

const PREVIEW_GROUP: &str = "toast-preview";
const OPEN_GROUP: &str = "toast-open";

fn hover_veil(kit: Kit) -> Rgba {
    rgba(qol_theme::translucent(
        kit.grounds.pane.ink,
        qol_theme::Alpha::Halo,
    ))
}

pub(super) fn ruled(card: impl IntoElement, left: f32, content: f32, scale: f32, kit: Kit) -> Div {
    div().size_full().relative().child(card).child(
        div()
            .absolute()
            .opacity(content)
            .top_0()
            .left_0()
            .w(relative(left.clamp(0.0, 1.0)))
            .h(px(qol_theme::toast::RULE * scale))
            .bg(rgb(kit.palette.accent)),
    )
}

pub(super) fn counting(row: &SlabSnapshotRow, paused: bool, now: Instant) -> Option<f32> {
    let timeout = row.toast.effective_timeout()?;
    let deadline = row.deadline.filter(|_| !paused)?;
    Some(deadline.saturating_duration_since(now).as_secs_f32() / timeout.as_secs_f32())
}

#[derive(Clone, Copy)]
pub(super) enum CardAct {
    Open,
    Preview,
    Close,
}

pub(super) trait CardHost {
    fn act(&self, id: RowId, act: CardAct, cx: &mut App);
}

pub(super) type Host = Rc<dyn CardHost>;

pub(super) fn row_ground(row: &SlabSnapshotRow, kit: Kit) -> u32 {
    if row.toast.live {
        kit.grounds.menu.bg
    } else {
        tone_ground(row.toast.tone, kit)
    }
}

pub(super) fn row_lift(row: &SlabSnapshotRow, kit: Kit) -> Rgba {
    rgb(qol_theme::lift(row_ground(row, kit), kit.grounds.pane.ink))
}

fn tone_ground(tone: ToastTone, kit: Kit) -> u32 {
    if tone == ToastTone::Danger {
        kit.grounds.invalid.bg
    } else {
        kit.grounds.pane.bg
    }
}

pub(super) fn age_label(age: Duration) -> String {
    let minutes = age.as_secs() / 60;
    match minutes {
        0 => "now".to_string(),
        1..=59 => format!("{minutes} min"),
        _ => format!("{} h", minutes / 60),
    }
}

pub(super) fn lone(row: &SlabSnapshotRow, host: Option<Host>, now: Instant) -> Div {
    let kit = crate::kit::kit();
    ruled(
        kit.window().bg(rgb(row_ground(row, kit))).child(content(
            row,
            CardParts {
                scale: 1.0,
                content: 1.0,
                interactive: host.is_some(),
                age: age_label(now.saturating_duration_since(row.created)),
            },
            kit,
            host,
        )),
        counting(row, false, now).unwrap_or(1.0),
        1.0,
        1.0,
        kit,
    )
}

pub(super) fn message(row: &SlabSnapshotRow, scale: f32, content: f32, left: f32, kit: Kit) -> Div {
    let ground = kit.grounds.pane;
    let line = |style: TextStyle, ink: u32, lines: usize, text: SharedString| {
        div()
            .w_full()
            .text_scaled(style, scale)
            .text_color(rgb(ink))
            .clamps(lines)
            .child(text)
    };
    let mut words = div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(qol_theme::SPACE_STACK * scale))
        .px(px(qol_theme::toast::MESSAGE_TEXT_INSET * scale))
        .text_center()
        .child(line(
            TextStyle::ListName,
            ground.ink,
            1,
            row.toast.title.clone(),
        ));
    if !row.toast.message.is_empty() {
        words = words.child(line(
            TextStyle::Detail,
            ground.soft,
            2,
            row.toast.message.clone(),
        ));
    }
    let below_rule = qol_theme::toast::RULE - qol_theme::LINE;
    let mut body = div()
        .size_full()
        .relative()
        .pt(px(below_rule * scale))
        .opacity(content)
        .child(words);
    if let Some(mark) = row.toast.mark {
        let inset = qol_theme::toast::MESSAGE_MARK_INSET;
        body = body.child(
            div()
                .absolute()
                .left(px(inset * scale))
                .top(px((inset + below_rule) * scale))
                .child(crate::icon::mark(
                    mark,
                    qol_theme::toast::MESSAGE_MARK * scale,
                    ground.faint,
                )),
        );
    }
    ruled(kit.window().child(body), left, content, scale, kit)
}

pub(super) struct CardParts {
    pub scale: f32,
    pub content: f32,
    pub interactive: bool,
    pub age: String,
}

pub(super) fn content(
    row: &SlabSnapshotRow,
    parts: CardParts,
    kit: Kit,
    host: Option<Host>,
) -> Div {
    let scale = parts.scale;
    let mut card = div()
        .size_full()
        .flex()
        .flex_row()
        .opacity(parts.content)
        .child(lead(row, scale, parts.interactive, kit, host.clone()))
        .child(text_zone(row, &parts, kit, host.clone()))
        .child(dismiss(row, &parts, kit, host.clone()));
    if let Some(host) = host.filter(|_| parts.interactive) {
        let id = row.id;
        card = card.on_mouse_down(MouseButton::Middle, move |_, _, cx| {
            host.act(id, CardAct::Close, cx)
        });
    }
    card
}

fn lead(
    row: &SlabSnapshotRow,
    scale: f32,
    interactive: bool,
    kit: Kit,
    host: Option<Host>,
) -> AnyElement {
    let Some(preview) = &row.toast.preview else {
        return div()
            .flex_none()
            .w(px(qol_theme::SPACE_PAD * scale))
            .into_any_element();
    };
    let slot = div()
        .flex_none()
        .h_full()
        .w(px(qol_theme::toast::PREVIEW * scale))
        .mr(px(qol_theme::SPACE_CELL * scale))
        .relative()
        .flex()
        .overflow_hidden()
        .child(preview.render(row.toast.tone.color(kit)));
    let Some(host) = host.filter(|_| interactive && row.toast.preview_action.is_some()) else {
        return slot.into_any_element();
    };
    let id = row.id;
    slot.id(("toast-preview", id.0))
        .group(PREVIEW_GROUP)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .group_hover(PREVIEW_GROUP, move |style| style.bg(hover_veil(kit))),
        )
        .cursor_pointer()
        .on_click(move |_, _, cx| host.act(id, CardAct::Preview, cx))
        .into_any_element()
}

fn text_zone(row: &SlabSnapshotRow, parts: &CardParts, kit: Kit, host: Option<Host>) -> AnyElement {
    let column = text_column(row, parts, kit);
    let Some(host) = host.filter(|_| parts.interactive && row.toast.activation.is_some()) else {
        return column.into_any_element();
    };
    let id = row.id;
    column
        .id(("toast-open", id.0))
        .group(OPEN_GROUP)
        .cursor_pointer()
        .on_click(move |_, _, cx| host.act(id, CardAct::Open, cx))
        .into_any_element()
}

fn text_column(row: &SlabSnapshotRow, parts: &CardParts, kit: Kit) -> Div {
    let scale = parts.scale;
    let ground = kit.grounds.pane;
    let source_line = div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .text_scaled(TextStyle::Label, scale)
        .text_color(rgb(ground.faint))
        .child(div().flex_none().child(SharedString::from(cased(
            TextStyle::Label,
            &row.toast.source,
        ))))
        .child(div().flex_grow())
        .child(
            div()
                .flex_none()
                .pr(px(qol_theme::SPACE_INSET * scale))
                .child(SharedString::from(cased(TextStyle::Label, &parts.age))),
        );
    let column = div()
        .flex_grow()
        .min_w_0()
        .overflow_hidden()
        .h_full()
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(qol_theme::SPACE_STACK * scale))
        .child(source_line)
        .child(
            div()
                .w_full()
                .text_scaled(TextStyle::ListName, scale)
                .text_color(rgb(ground.ink))
                .child(row.toast.title.clone()),
        );
    if row.toast.message.is_empty() {
        return column;
    }
    let opens = parts.interactive && row.toast.activation.is_some();
    let detail = |ink: u32| {
        if row.toast.message_is_path {
            let (head, tail) = crate::kit::path_label(&row.toast.message);
            path_line(head, tail, scale, ink)
        } else {
            div()
                .w_full()
                .text_scaled(TextStyle::Detail, scale)
                .clamps(2)
                .text_color(rgb(ink))
                .child(row.toast.message.clone())
        }
    };
    if !opens {
        return column.child(detail(ground.soft));
    }
    column.child(
        div().w_full().relative().child(detail(ground.soft)).child(
            detail(kit.palette.accent)
                .absolute()
                .top_0()
                .left_0()
                .opacity(0.0)
                .group_hover(OPEN_GROUP, |style| style.opacity(1.0)),
        ),
    )
}

fn path_line(head: String, tail: String, scale: f32, ink: u32) -> Div {
    let piece = |text: String| {
        div()
            .min_w_0()
            .flex_grow()
            .text_scaled(TextStyle::Code, scale)
            .text_color(rgb(ink))
            .child(SharedString::from(text))
    };
    let mut line = div().w_full().flex().flex_row().overflow_hidden();
    if !head.is_empty() {
        line = line.child(piece(head));
    }
    line.child(piece(tail))
}

fn dismiss(row: &SlabSnapshotRow, parts: &CardParts, kit: Kit, host: Option<Host>) -> AnyElement {
    let scale = parts.scale;
    let side = qol_theme::toast::CLOSE * scale;
    let control = div()
        .id(("toast-dismiss", row.id.0))
        .flex_none()
        .w(px(side))
        .h(px(side))
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .child(crate::icon::icon(
            crate::Icon::Close,
            qol_theme::TEXT_MICRO * scale,
            kit.grounds.pane.soft,
        ));
    let Some(host) = host.filter(|_| parts.interactive) else {
        return control.into_any_element();
    };
    let id = row.id;
    control
        .hover(move |style| style.bg(hover_veil(kit)))
        .cursor_pointer()
        .on_click(move |_, _, cx| host.act(id, CardAct::Close, cx))
        .into_any_element()
}

pub(super) fn strip(
    count: usize,
    scale: f32,
    kit: Kit,
    clear_all: impl Fn(&mut App) + 'static,
) -> Div {
    let ground = kit.grounds.pane;
    div()
        .size_full()
        .flex()
        .flex_row()
        .items_center()
        .pl(px(qol_theme::SPACE_CELL * scale))
        .text_scaled(TextStyle::Detail, scale)
        .text_color(rgb(ground.faint))
        .child(SharedString::from(format!("{count} notifications")))
        .child(div().flex_grow())
        .child(
            crate::kit::kit()
                .pointable(
                    div().id("toast-clear-all"),
                    rgb(qol_theme::lift(ground.bg, ground.ink)),
                )
                .h_full()
                .px(px(qol_theme::SPACE_CELL * scale))
                .flex()
                .items_center()
                .cursor_pointer()
                .text_color(rgb(ground.soft))
                .child(SharedString::from("Clear all"))
                .on_click(move |_, _, cx| clear_all(cx)),
        )
}

pub(super) fn show_all(words: f32, kit: Kit) -> Div {
    div()
        .w_full()
        .h(px(super::pile::WORDS_ROW))
        .mt(px((1.0 - words) * super::pile::WORDS_RISE))
        .flex()
        .items_center()
        .justify_center()
        .opacity(words)
        .child(
            div()
                .px(px(qol_theme::SPACE_INSET))
                .py(px(qol_theme::SPACE_STACK))
                .bg(rgba(qol_theme::translucent(
                    kit.grounds.pane.bg,
                    qol_theme::Alpha::Strong,
                )))
                .text(TextStyle::Label)
                .text_color(rgb(kit.grounds.pane.faint))
                .child(SharedString::from(cased(TextStyle::Label, "Show all"))),
        )
}

pub(super) fn card_id(id: RowId) -> (&'static str, u64) {
    ("toast-card", id.0)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::age_label;

    #[test]
    fn ages_read_now_then_minutes_then_hours() {
        let cases = [
            (0, "now"),
            (59, "now"),
            (60, "1 min"),
            (3599, "59 min"),
            (7200, "2 h"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(age_label(Duration::from_secs(seconds)), expected);
        }
    }
}
