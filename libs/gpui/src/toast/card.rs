use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::*;
use qol_theme::{Ground, TextStyle};

use super::{RowId, SlabSnapshotRow, Toast, ToastTone};
use crate::kit::Kit;
use crate::text::{cased, TextStyled};

const OPEN_GROUP: &str = "toast-open";
const DETAIL_LINES: usize = 2;

fn hover_veil(kit: Kit) -> Rgba {
    rgba(qol_theme::translucent(
        kit.grounds.pane.ink,
        qol_theme::Alpha::Halo,
    ))
}

pub(super) fn ruled(card: impl IntoElement, left: f32, content: f32, scale: f32, mark: u32) -> Div {
    div().size_full().relative().child(card).child(
        div()
            .absolute()
            .opacity(content)
            .top_0()
            .left_0()
            .w(relative(left.clamp(0.0, 1.0)))
            .h(px(qol_theme::toast::RULE * scale))
            .bg(rgb(mark)),
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
    Action(usize),
    Close,
}

pub(super) trait CardHost {
    fn act(&self, id: RowId, act: CardAct, cx: &mut App);
}

pub(super) type Host = Rc<dyn CardHost>;

pub(super) fn row_tone(row: &SlabSnapshotRow, kit: Kit) -> Ground {
    if row.toast.live {
        kit.grounds.menu
    } else {
        tone_ground(row.toast.tone, kit)
    }
}

pub(super) fn row_ground(row: &SlabSnapshotRow, kit: Kit) -> u32 {
    row_tone(row, kit).bg
}

pub(super) fn row_lift(row: &SlabSnapshotRow, kit: Kit) -> Rgba {
    rgb(qol_theme::lift(row_ground(row, kit), kit.grounds.pane.ink))
}

fn tone_ground(tone: ToastTone, kit: Kit) -> Ground {
    match tone {
        ToastTone::Danger => kit.grounds.invalid,
        ToastTone::Warning => kit.grounds.attention,
        ToastTone::Neutral | ToastTone::Info | ToastTone::Success => kit.grounds.pane,
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

fn line_of(style: TextStyle) -> f32 {
    let spec = style.spec();
    spec.size * spec.line_height
}

fn detail_width(toast: &Toast) -> f32 {
    let lead = if toast.preview.is_some() {
        qol_theme::toast::PREVIEW + qol_theme::SPACE_CELL
    } else {
        qol_theme::SPACE_PAD
    };
    super::pile::CARD_WIDTH - lead - qol_theme::toast::CLOSE
}

fn words_height(toast: &Toast, detail_lines: usize) -> f32 {
    let detail = if toast.message.is_empty() {
        0.0
    } else if toast.message_is_path {
        qol_theme::SPACE_STACK + line_of(TextStyle::Code)
    } else {
        qol_theme::SPACE_STACK
            + line_of(TextStyle::Detail) * detail_lines.clamp(1, DETAIL_LINES) as f32
    };
    2.0 * qol_theme::SPACE_CELL
        + qol_theme::toast::MARK.max(line_of(TextStyle::Detail))
        + qol_theme::SPACE_STACK
        + line_of(TextStyle::Title)
        + detail
}

/// The card's height for its words: the source line, the title, the detail
/// lines, the window's border and the button bar, never less than the
/// resting card.
pub(super) fn fit_height(toast: &Toast, detail_lines: usize) -> f32 {
    use qol_theme::toast::{ACTION_BAR, HEIGHT, HEIGHT_STEP};
    let framed = words_height(toast, detail_lines) + 2.0 * qol_theme::LINE;
    let body = ((framed / HEIGHT_STEP).ceil() * HEIGHT_STEP).max(HEIGHT);
    if toast.actions.is_empty() {
        body
    } else {
        body + ACTION_BAR
    }
}

pub(super) fn measured_height(toast: &Toast, window: &mut Window) -> f32 {
    let lines = if toast.message.is_empty() || toast.message_is_path {
        1
    } else {
        let spec = TextStyle::Detail.spec();
        crate::text::wrapped_line_count(
            window,
            &toast.message,
            font(spec.face.family()),
            spec.size,
            detail_width(toast),
        )
    };
    fit_height(toast, lines)
}

pub(super) fn lone(row: &SlabSnapshotRow, host: Option<Host>, now: Instant) -> Div {
    let kit = crate::kit::kit();
    let ground = row_tone(row, kit);
    ruled(
        kit.window().bg(rgb(ground.bg)).child(content(
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
        ground.mark,
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
    ruled(kit.window().child(body), left, content, scale, ground.mark)
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
    let ground = row_tone(row, kit);
    let body = div()
        .flex_grow()
        .min_h_0()
        .w_full()
        .flex()
        .flex_row()
        .child(lead(row, scale, parts.interactive, kit, host.clone()))
        .child(text_zone(row, &parts, ground, kit, host.clone()));
    let mut card = div()
        .size_full()
        .relative()
        .flex()
        .flex_col()
        .opacity(parts.content)
        .child(body);
    if !row.toast.actions.is_empty() {
        card = card.child(action_bar(row, &parts, ground, kit, host.clone()));
    }
    card = card.child(dismiss(row, &parts, ground, kit, host.clone()));
    if let Some(host) = host.filter(|_| parts.interactive) {
        let id = row.id;
        card = card.on_mouse_down(MouseButton::Middle, move |_, window, cx| {
            host.act(id, CardAct::Close, cx);
            window.refresh();
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
        .child(kit.pointable(
            div().absolute().top_0().left_0().size_full(),
            hover_veil(kit),
        ))
        .cursor_pointer()
        .on_click(move |_, _, cx| host.act(id, CardAct::Preview, cx))
        .into_any_element()
}

fn text_zone(
    row: &SlabSnapshotRow,
    parts: &CardParts,
    ground: Ground,
    kit: Kit,
    host: Option<Host>,
) -> AnyElement {
    let column = text_column(row, parts, ground, kit);
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

fn source_line(row: &SlabSnapshotRow, parts: &CardParts, ground: Ground) -> Div {
    let scale = parts.scale;
    let mut line = div()
        .w_full()
        .flex_none()
        .h(px(qol_theme::toast::MARK * scale))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_INSET * scale))
        .text_scaled(TextStyle::Detail, scale)
        .text_color(rgb(ground.faint));
    if let Some(mark) = row.toast.mark {
        line = line.child(div().flex_none().child(crate::icon::mark(
            mark,
            qol_theme::toast::MARK * scale,
            ground.faint,
        )));
    }
    line.child(
        div()
            .min_w_0()
            .flex_shrink()
            .child(row.toast.source.clone()),
    )
    .child(div().flex_grow())
    .child(
        div()
            .flex_none()
            .child(SharedString::from(parts.age.clone())),
    )
}

fn text_column(row: &SlabSnapshotRow, parts: &CardParts, ground: Ground, kit: Kit) -> Div {
    let scale = parts.scale;
    let column = div()
        .flex_grow()
        .min_w_0()
        .overflow_hidden()
        .h_full()
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(qol_theme::SPACE_STACK * scale))
        .py(px(qol_theme::SPACE_CELL * scale))
        .pr(px(qol_theme::toast::CLOSE * scale))
        .child(source_line(row, parts, ground))
        .child(
            div()
                .w_full()
                .text_scaled(TextStyle::Title, scale)
                .truncate()
                .text_color(rgb(ground.ink))
                .child(row.toast.title.clone()),
        );
    if row.toast.message.is_empty() {
        return column;
    }
    let opens = parts.interactive && row.toast.activation.is_some();
    let detail = |ink: u32| {
        if row.toast.message_is_path {
            div()
                .w_full()
                .text_scaled(TextStyle::Code, scale)
                .truncate()
                .text_color(rgb(ink))
                .child(file_name(&row.toast.message))
        } else {
            div()
                .w_full()
                .text_scaled(TextStyle::Detail, scale)
                .clamps(DETAIL_LINES)
                .text_color(rgb(ink))
                .child(row.toast.message.clone())
        }
    };
    if !opens {
        return column.child(detail(ground.soft));
    }
    column.child(
        div()
            .w_full()
            .relative()
            .child(detail(ground.soft))
            .child(kit.revealed_by(detail(ground.mark).absolute().top_0().left_0(), OPEN_GROUP)),
    )
}

fn file_name(path: &str) -> SharedString {
    let name = std::path::Path::new(path).file_name().map_or_else(
        || path.to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    SharedString::from(name)
}

fn action_bar(
    row: &SlabSnapshotRow,
    parts: &CardParts,
    ground: Ground,
    kit: Kit,
    host: Option<Host>,
) -> Div {
    let scale = parts.scale;
    let host = host.filter(|_| parts.interactive);
    let mut bar = div()
        .flex_none()
        .w_full()
        .h(px(qol_theme::toast::ACTION_BAR * scale))
        .flex()
        .flex_row()
        .gap(px(qol_theme::LINE))
        .border_t(px(qol_theme::LINE))
        .border_color(rgba(ground.edge.packed()));
    for (index, action) in row.toast.actions.iter().enumerate() {
        let (fill, ink) = if action.happy {
            (Some(ground.mark), ground.on_mark)
        } else {
            (None, ground.soft)
        };
        let mut cell = div()
            .id(SharedString::from(format!(
                "toast-action-{}-{index}",
                row.id.0
            )))
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .px(px(qol_theme::SPACE_PAD * scale))
            .text_scaled(TextStyle::ListName, scale)
            .text_color(rgb(ink))
            .child(action.label.clone());
        if let Some(fill) = fill {
            cell = cell.bg(rgb(fill));
        }
        if let Some(host) = host.clone() {
            let id = row.id;
            let lift = qol_theme::lift(fill.unwrap_or(ground.bg), ground.ink);
            cell = kit
                .pointable(cell, rgb(lift))
                .cursor_pointer()
                .on_click(move |_, _, cx| host.act(id, CardAct::Action(index), cx));
        }
        bar = bar.child(cell);
    }
    bar
}

fn dismiss(
    row: &SlabSnapshotRow,
    parts: &CardParts,
    ground: Ground,
    kit: Kit,
    host: Option<Host>,
) -> AnyElement {
    let scale = parts.scale;
    let control = div()
        .id(("toast-dismiss", row.id.0))
        .absolute()
        .top_0()
        .right_0()
        .w(px(qol_theme::toast::CLOSE * scale))
        .h(px(qol_theme::toast::CLOSE_HEIGHT * scale))
        .flex()
        .items_center()
        .justify_center()
        .child(crate::icon::icon(
            crate::Icon::Close,
            qol_theme::TEXT_MICRO * scale,
            ground.soft,
        ));
    let Some(host) = host.filter(|_| parts.interactive) else {
        return control.into_any_element();
    };
    let id = row.id;
    kit.pointable(control, hover_veil(kit))
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

    use super::{age_label, fit_height, tone_ground, words_height};
    use crate::toast::{Toast, ToastLayout, ToastTone};

    #[test]
    fn a_card_is_as_tall_as_its_words_and_never_below_the_resting_card() {
        let rest = qol_theme::toast::HEIGHT;
        let bare = Toast::new("Saved", "", ToastLayout::default());
        assert_eq!(fit_height(&bare, 1), rest);
        let plain = Toast::new("Saved", "One line of detail", ToastLayout::default());
        for lines in [1, 2] {
            let inside = fit_height(&plain, lines) - 2.0 * qol_theme::LINE;
            assert!(
                inside >= words_height(&plain, lines),
                "{lines} detail lines must fit inside the border"
            );
        }
        let short = fit_height(&plain, 1);
        assert_eq!(short % qol_theme::toast::HEIGHT_STEP, 0.0);
        let long = fit_height(&plain, 2);
        assert!(long > short, "a second detail line grows the card");
        assert_eq!(long % qol_theme::toast::HEIGHT_STEP, 0.0);
        assert_eq!(
            fit_height(&plain, 9),
            long,
            "the detail clamps to two lines"
        );
        let asking = plain.action("Open", |_| Ok(()));
        assert_eq!(fit_height(&asking, 1), short + qol_theme::toast::ACTION_BAR);
    }

    #[test]
    fn warnings_and_errors_sit_on_their_own_grounds() {
        let kit = crate::kit::Kit::new(qol_theme::ThemeMode::Dark, qol_theme::DARK_SYSTEM);
        assert_eq!(tone_ground(ToastTone::Danger, kit), kit.grounds.invalid);
        assert_eq!(tone_ground(ToastTone::Warning, kit), kit.grounds.attention);
        for tone in [ToastTone::Neutral, ToastTone::Info, ToastTone::Success] {
            assert_eq!(tone_ground(tone, kit), kit.grounds.pane);
        }
        assert_ne!(kit.grounds.invalid.mark, kit.palette.accent);
    }

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
