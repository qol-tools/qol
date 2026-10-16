use std::path::PathBuf;

use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_gpui::icon::mark;
use qol_gpui::kit::Chip;
use qol_gpui::scrollbar::{overflow_fade, OverflowEdge};
use qol_gpui::text::{shaped_width, wrapped_line_count, TextStyled};
use qol_gpui::theme::{
    clear, solid, Ground, Mark, TextStyle, LINE, RADIUS_CONTROL, RADIUS_TIGHT, SPACE_INSET,
    SPACE_SNUG, SPACE_STACK,
};
use qol_plugin_api::launcher_flows::{DetailText, FlowDetail, FlowRow as RowSpec};

use super::super::about::BAND_INSET;
use super::super::view::launcher_mark;
use crate::flow::FlowEntry;

pub const CODE_LINE: f32 = 18.0;
pub const GUTTER: f32 = 48.0;
const GUTTER_AIR: f32 = 10.0;
const FADE: f32 = 36.0;
const WELL_INSET: f32 = 10.0;
const LINK_TITLE_TOP: f32 = 56.0;
const MASK_DOTS: usize = 12;
const TAIL_SHARE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Detail<'a> {
    Inline,
    Loading,
    Ready(&'a FlowDetail),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Text(DetailText),
    Loading,
    Image(PathBuf),
    Swatch { fill: u32, value: String },
    Link { url: String, title: Option<String> },
    Masked,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Text { number: Option<usize>, text: String },
    Hidden(usize),
}

pub fn body_of(row: &crate::flow::FlowRow, whole: bool, detail: Detail) -> Body {
    let spec = &row.spec;
    if spec.masked {
        return Body::Masked;
    }
    if let Some(fill) = swatch_of(spec) {
        return Body::Swatch {
            fill,
            value: row.copy.clone().unwrap_or_else(|| row.title.clone()),
        };
    }
    if let Some(path) = spec.thumb_path() {
        return Body::Image(path);
    }
    let excerpt = match detail {
        Detail::Ready(detail) => detail.text.as_ref(),
        Detail::Inline | Detail::Loading => None,
    };
    if spec.kind.as_deref() == Some("link") {
        return Body::Link {
            url: excerpt
                .and_then(|text| text.head.first().cloned())
                .or_else(|| row.copy.clone())
                .unwrap_or_else(|| row.title.clone()),
            title: spec.preview.as_ref().map(|preview| preview.text.clone()),
        };
    }
    if let Some(text) = excerpt {
        return Body::Text(text.clone());
    }
    if detail == Detail::Loading {
        return Body::Loading;
    }
    match &spec.preview {
        Some(preview) => Body::Text(whole_text(
            match (&row.copy, whole && preview.mono) {
                (Some(copy), true) => copy,
                _ => &preview.text,
            },
            preview.mono,
        )),
        None => Body::Text(whole_text(
            row.copy.as_deref().unwrap_or(&row.title),
            spec.mono,
        )),
    }
}

pub fn whole_text(text: &str, numbered: bool) -> DetailText {
    let head: Vec<String> = text.lines().map(str::to_string).collect();
    DetailText {
        lines: head.len(),
        chars: text.chars().count(),
        head,
        tail: Vec::new(),
        numbered,
    }
}

pub fn swatch_of(spec: &RowSpec) -> Option<u32> {
    let (red, green, blue) = qol_color::parse_hex_color(spec.swatch.as_deref()?)?;
    Some(u32::from_be_bytes([0, red, green, blue]))
}

pub fn masked_text() -> String {
    "\u{2022}".repeat(MASK_DOTS)
}

pub fn fits(height: f32) -> usize {
    ((height - 2.0 * SPACE_INSET) / CODE_LINE).floor().max(1.0) as usize
}

pub fn numbered_lines(text: &DetailText, fit: usize) -> (Vec<Line>, bool) {
    let lines = text
        .head
        .iter()
        .take(fit + 1)
        .enumerate()
        .map(|(at, line)| Line::Text {
            number: Some(at + 1),
            text: line.clone(),
        })
        .collect();
    (lines, text.lines > fit)
}

pub fn head_and_tail(text: &DetailText, fit: usize) -> Vec<Line> {
    let numbered = |number: usize| {
        known_line(text, number).map(|line| Line::Text {
            number: Some(number),
            text: line.to_string(),
        })
    };
    if text.lines <= fit {
        return (1..=text.lines).filter_map(numbered).collect();
    }
    let tail = (fit / TAIL_SHARE).max(1);
    let head = fit.saturating_sub(tail + 1);
    let hidden = text.lines - head - tail;
    (1..=head)
        .filter_map(numbered)
        .chain(std::iter::once(Line::Hidden(hidden)))
        .chain((text.lines - tail + 1..=text.lines).filter_map(numbered))
        .collect()
}

fn known_line(text: &DetailText, number: usize) -> Option<&str> {
    if (1..=text.head.len()).contains(&number) {
        return Some(&text.head[number - 1]);
    }
    let start = text.tail_start();
    number
        .checked_sub(start)
        .and_then(|at| text.tail.get(at))
        .map(String::as_str)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Counted {
    pub lines: usize,
    pub numbered: bool,
}

impl Body {
    pub fn counted(&self) -> Option<Counted> {
        match self {
            Body::Text(text) => Some(Counted {
                lines: text.lines,
                numbered: text.numbered,
            }),
            _ => None,
        }
    }
}

pub fn shown(counted: Counted, fit: usize, over: bool) -> String {
    match (over, counted.numbered) {
        (false, _) => ", all shown".to_string(),
        (true, true) => format!(" \u{b7} {} shown", grouped(fit.min(counted.lines))),
        (true, false) => " \u{b7} more below".to_string(),
    }
}

pub fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub fn tile(
    row: &crate::flow::FlowRow,
    entry: &FlowEntry,
    size: f32,
    glyph: f32,
    ground: Ground,
) -> Div {
    let spec = &row.spec;
    let well = div()
        .flex_none()
        .size(px(size))
        .rounded(px(RADIUS_CONTROL))
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center();
    if !spec.masked {
        if let Some(fill) = swatch_of(spec) {
            return well.bg(rgb(fill));
        }
        if let Some(path) = spec.thumb_path() {
            return well
                .bg(rgba(ground.well.packed()))
                .child(covered(path, size, size));
        }
    }
    let well = well.bg(rgba(ground.well.packed()));
    match kind_mark(spec, entry) {
        Some(drawn) => well.child(mark(drawn, glyph, Mark::line_on(&ground))),
        None => well
            .text(TextStyle::Name)
            .text_color(rgb(ground.soft))
            .child(
                row.title
                    .chars()
                    .find(|character| character.is_alphanumeric())
                    .map_or_else(
                        || "\u{2022}".to_owned(),
                        |character| character.to_uppercase().to_string(),
                    ),
            ),
    }
}

fn kind_mark(spec: &RowSpec, entry: &FlowEntry) -> Option<Mark> {
    spec.kind
        .as_deref()
        .and_then(mark_of_kind)
        .or_else(|| entry.icon.as_deref().and_then(launcher_mark))
}

fn mark_of_kind(kind: &str) -> Option<Mark> {
    match kind {
        "text" => Some(Mark::ClipText),
        "code" | "command" | "json" => Some(Mark::ClipCode),
        "image" => Some(Mark::ClipImage),
        "file" => Some(Mark::ClipFile),
        "secret" => Some(Mark::ClipSecret),
        "color" => Some(Mark::ClipColor),
        "link" => Some(Mark::Link),
        other => Mark::from_name(other),
    }
}

fn covered(path: PathBuf, width: f32, height: f32) -> Div {
    let picture = img(path.clone()).absolute().left_0().top_0();
    let picture = match imagesize::size(&path) {
        Ok(image) => {
            let (cover_width, cover_height) =
                cover_size((image.width as f32, image.height as f32), width, height);
            picture
                .w(px(cover_width))
                .h(px(cover_height))
                .object_fit(ObjectFit::Fill)
        }
        Err(_) => picture.w_full().h_full().object_fit(ObjectFit::Cover),
    };
    div().absolute().inset_0().overflow_hidden().child(picture)
}

fn cover_size(image: (f32, f32), width: f32, height: f32) -> (f32, f32) {
    let (image_width, image_height) = image;
    if image_width <= 0.0 || image_height <= 0.0 {
        return (width, height);
    }
    let scale = (width / image_width).max(height / image_height);
    (image_width * scale, image_height * scale)
}

pub struct Frame {
    pub body: Body,
    pub height: f32,
    pub width: f32,
    pub whole: bool,
}

pub fn frame(frame: Frame, window: &mut Window) -> (Div, bool) {
    let kit = qol_gpui::kit::kit();
    let pane = kit.grounds.pane;
    let fit = fits(frame.height);
    let shell = div()
        .absolute()
        .left(px(BAND_INSET))
        .right(px(BAND_INSET))
        .h(px(frame.height))
        .overflow_hidden()
        .rounded(px(RADIUS_TIGHT))
        .bg(rgb(pane.bg))
        .border_b(px(LINE))
        .border_color(rgba(kit.washes.hairline.packed()));
    let (inner, fade) = match frame.body {
        Body::Masked => (
            code_column(
                vec![Line::Text {
                    number: None,
                    text: masked_text(),
                }],
                GUTTER_AIR,
            ),
            false,
        ),
        Body::Loading => (div(), false),
        Body::Text(text) if text.numbered => {
            if frame.whole {
                let over = text.lines > fit;
                let lines = head_and_tail(&text, fit);
                let air = gutter_air(window, &lines);
                (code_column(lines, air), over)
            } else {
                let (lines, over) = numbered_lines(&text, fit);
                let air = gutter_air(window, &lines);
                (code_column(lines, air), over)
            }
        }
        Body::Text(text) => {
            let width = frame.width - 2.0 * BAND_INSET - GUTTER - BAND_INSET;
            let prose = text.head.join("\n");
            let mut used = 0;
            for paragraph in prose.lines() {
                if used > fit {
                    break;
                }
                used += wrapped_line_count(
                    window,
                    paragraph,
                    font(qol_gpui::theme::font_mono()),
                    TextStyle::Code.spec().size,
                    width,
                )
                .max(1);
            }
            let cut = text.lines > text.head.len();
            (prose_column(&prose), cut || used > fit)
        }
        Body::Image(path) => (
            covered(path, frame.width - 2.0 * BAND_INSET, frame.height),
            false,
        ),
        Body::Swatch { fill, value } => (
            div().absolute().inset_0().bg(rgb(fill)).child(
                div()
                    .absolute()
                    .left(px(WELL_INSET))
                    .bottom(px(WELL_INSET))
                    .child(kit.chip(Chip::Tag(value.into()), pane)),
            ),
            false,
        ),
        Body::Link { url, title } => (
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .left(px(WELL_INSET))
                        .right(px(WELL_INSET))
                        .top(px(WELL_INSET))
                        .text(TextStyle::Code)
                        .text_color(rgb(pane.soft))
                        .wraps()
                        .child(url),
                )
                .when_some(title, |body, title| {
                    body.child(
                        div()
                            .absolute()
                            .left(px(WELL_INSET))
                            .right(px(WELL_INSET))
                            .top(px(LINK_TITLE_TOP))
                            .text(TextStyle::Value)
                            .text_color(rgb(pane.ink))
                            .wraps()
                            .child(title),
                    )
                }),
            false,
        ),
    };
    let shell = shell.child(inner).when(fade, |shell| {
        shell.child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(FADE))
                .bg(overflow_fade(
                    OverflowEdge::Bottom,
                    solid(pane.bg),
                    clear(pane.bg),
                )),
        )
    });
    (shell, fade)
}

// A number too wide for the gutter takes its air, so it never crosses the frame's edge.
fn gutter_air(window: &mut Window, lines: &[Line]) -> f32 {
    let Some(widest) = lines
        .iter()
        .filter_map(|line| match line {
            Line::Text { number, .. } => *number,
            _ => None,
        })
        .max()
    else {
        return GUTTER_AIR;
    };
    let digits = shaped_width(
        window,
        &widest.to_string(),
        font(qol_gpui::theme::font_mono()),
        TextStyle::Code.spec().size,
    );
    (GUTTER - digits).clamp(0.0, GUTTER_AIR)
}

fn code_column(lines: Vec<Line>, air: f32) -> Div {
    let pane = qol_gpui::kit::kit().grounds.pane;
    div()
        .absolute()
        .inset_0()
        .pt(px(SPACE_INSET))
        .pr(px(BAND_INSET))
        .flex()
        .flex_col()
        .overflow_hidden()
        .text(TextStyle::Code)
        .children(lines.into_iter().map(move |line| {
            match line {
                Line::Text { number, text } => div()
                    .flex_none()
                    .h(px(CODE_LINE))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_none()
                            .w(px(GUTTER))
                            .pr(px(air))
                            .flex()
                            .justify_end()
                            .text_color(rgb(pane.faint))
                            .children(number.map(|number| number.to_string())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(rgb(pane.soft))
                            .child(text),
                    ),
                Line::Hidden(count) => hidden_divider(count),
            }
        }))
}

fn hidden_divider(count: usize) -> Div {
    let kit = qol_gpui::kit::kit();
    let rule = || {
        div()
            .flex_1()
            .h(px(LINE))
            .bg(rgba(kit.washes.hairline_strong.packed()))
    };
    div()
        .flex_none()
        .h(px(CODE_LINE))
        .my(px(SPACE_SNUG))
        .pl(px(GUTTER_AIR))
        .flex()
        .items_center()
        .gap(px(SPACE_INSET))
        .text(TextStyle::Detail)
        .text_color(rgb(kit.grounds.pane.faint))
        .child(rule())
        .child(format!("{} lines", grouped(count)))
        .child(rule())
}

fn prose_column(text: &str) -> Div {
    let pane = qol_gpui::kit::kit().grounds.pane;
    div()
        .absolute()
        .inset_0()
        .pt(px(SPACE_INSET))
        .pl(px(GUTTER))
        .pr(px(BAND_INSET))
        .flex()
        .flex_col()
        .gap(px(SPACE_INSET))
        .overflow_hidden()
        .text(TextStyle::Code)
        .text_color(rgb(pane.soft))
        .children(
            text.lines()
                .filter(|paragraph| !paragraph.trim().is_empty())
                .map(|paragraph| div().flex_none().wraps().child(paragraph.to_string())),
        )
        .child(div().h(px(SPACE_STACK)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use qol_plugin_api::launcher_flows::Preview;

    fn text(lines: &[&str], numbered: bool) -> Body {
        Body::Text(whole_text(&lines.join("\n"), numbered))
    }

    fn log(lines: usize) -> String {
        (1..=lines)
            .map(|at| format!("l{at}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn log_excerpt(lines: usize, head: usize, tail: usize) -> DetailText {
        DetailText {
            head: (1..=head).map(|at| format!("l{at}")).collect(),
            tail: (lines - tail + 1..=lines)
                .map(|at| format!("l{at}"))
                .collect(),
            lines,
            chars: 0,
            numbered: true,
        }
    }

    fn row(spec: RowSpec, copy: Option<&str>) -> crate::flow::FlowRow {
        crate::flow::FlowRow {
            title: spec.title.clone(),
            subtitle: None,
            copy: copy.map(str::to_string),
            raw: serde_json::json!({}),
            spec,
        }
    }

    #[test]
    fn every_kind_picks_one_body() {
        let base = RowSpec {
            title: "t".to_string(),
            ..RowSpec::default()
        };
        let preview = |text: &str| {
            Some(Preview {
                text: text.to_string(),
                mono: false,
            })
        };
        let cases = vec![
            (
                RowSpec {
                    masked: true,
                    swatch: Some("#d9822b".to_string()),
                    ..base.clone()
                },
                Some("secret"),
                Body::Masked,
            ),
            (
                RowSpec {
                    swatch: Some("#d9822b".to_string()),
                    ..base.clone()
                },
                Some("#d9822b"),
                Body::Swatch {
                    fill: 0xd9822b,
                    value: "#d9822b".to_string(),
                },
            ),
            (
                RowSpec {
                    thumb: Some("file:///thumbs/c.webp".to_string()),
                    ..base.clone()
                },
                None,
                Body::Image(PathBuf::from("/thumbs/c.webp")),
            ),
            (
                RowSpec {
                    kind: Some("link".to_string()),
                    preview: preview("fix(monitor)"),
                    ..base.clone()
                },
                Some("https://github.com"),
                Body::Link {
                    url: "https://github.com".to_string(),
                    title: Some("fix(monitor)".to_string()),
                },
            ),
            (
                RowSpec {
                    mono: true,
                    ..base.clone()
                },
                Some("ssh mint@host"),
                text(&["ssh mint@host"], true),
            ),
            (
                RowSpec {
                    preview: preview("prose"),
                    ..base
                },
                Some("prose copy"),
                text(&["prose"], false),
            ),
        ];
        for (spec, copy, expected) in cases {
            assert_eq!(body_of(&row(spec, copy), false, Detail::Inline), expected);
        }
    }

    #[test]
    fn a_flow_with_a_detail_query_draws_its_detail_or_an_empty_frame() {
        let spec = RowSpec {
            title: "l1".to_string(),
            kind: Some("text".to_string()),
            ..RowSpec::default()
        };
        let text_row = row(spec.clone(), None);
        assert_eq!(body_of(&text_row, false, Detail::Loading), Body::Loading);
        let excerpt = log_excerpt(1_284, 40, 12);
        let detail = FlowDetail {
            text: Some(excerpt.clone()),
            shown_fact: Some("Size".to_string()),
        };
        assert_eq!(
            body_of(&text_row, true, Detail::Ready(&detail)),
            Body::Text(excerpt)
        );
        assert_eq!(
            body_of(&text_row, false, Detail::Ready(&FlowDetail::default())),
            text(&["l1"], false)
        );
        let link = crate::flow::FlowRow {
            spec: RowSpec {
                kind: Some("link".to_string()),
                ..spec
            },
            ..text_row
        };
        let long_url = FlowDetail {
            text: Some(whole_text("https://example.com/a-very-long-path", true)),
            shown_fact: None,
        };
        assert_eq!(
            body_of(&link, false, Detail::Ready(&long_url)),
            Body::Link {
                url: "https://example.com/a-very-long-path".to_string(),
                title: None,
            }
        );
        let secret = RowSpec {
            masked: true,
            ..link.spec.clone()
        };
        assert_eq!(
            body_of(&row(secret, None), false, Detail::Loading),
            Body::Masked
        );
    }

    #[test]
    fn an_image_covers_its_box_from_the_top_left() {
        assert_eq!(cover_size((800.0, 600.0), 300.0, 300.0), (400.0, 300.0));
        assert_eq!(cover_size((200.0, 100.0), 44.0, 44.0), (88.0, 44.0));
        assert_eq!(cover_size((100.0, 400.0), 44.0, 44.0), (44.0, 176.0));
        assert_eq!(cover_size((0.0, 10.0), 44.0, 44.0), (44.0, 44.0));
    }

    #[test]
    fn every_row_kind_draws_its_own_mark() {
        let cases = [
            ("text", Some(Mark::ClipText)),
            ("code", Some(Mark::ClipCode)),
            ("command", Some(Mark::ClipCode)),
            ("json", Some(Mark::ClipCode)),
            ("image", Some(Mark::ClipImage)),
            ("file", Some(Mark::ClipFile)),
            ("secret", Some(Mark::ClipSecret)),
            ("color", Some(Mark::ClipColor)),
            ("link", Some(Mark::Link)),
            ("memory", Some(Mark::Memory)),
            ("answer", None),
        ];
        for (kind, expected) in cases {
            assert_eq!(mark_of_kind(kind), expected, "{kind}");
        }
    }

    #[test]
    fn swatches_reject_what_they_cannot_show() {
        let spec = |swatch: &str, thumb: &str| RowSpec {
            title: "t".to_string(),
            swatch: Some(swatch.to_string()),
            thumb: Some(thumb.to_string()),
            ..RowSpec::default()
        };
        assert_eq!(swatch_of(&spec("#fff", "relative.png")), None);
        assert_eq!(swatch_of(&spec("00ff00", "/a.png")), Some(0x00ff00));
    }

    #[test]
    fn the_panel_shows_one_line_past_the_fit_and_flags_the_fade() {
        let thirty = (1..=30)
            .map(|at| format!("line {at}"))
            .collect::<Vec<_>>()
            .join("\n");
        let fit = fits(322.0);
        assert_eq!(fit, 17);
        let (lines, over) = numbered_lines(&whole_text(&thirty, true), fit);
        assert!(over);
        assert_eq!(lines.len(), 18);
        assert_eq!(
            lines[0],
            Line::Text {
                number: Some(1),
                text: "line 1".to_string()
            }
        );
        let (lines, over) = numbered_lines(&whole_text("a\nb", true), fit);
        assert!(!over);
        assert_eq!(lines.len(), 2);
        let (lines, over) = numbered_lines(&log_excerpt(1_284, 40, 12), fit);
        assert!(over);
        assert_eq!(lines.len(), 18);
    }

    #[test]
    fn an_inline_log_row_shows_its_real_tail_in_the_sheet() {
        let log = log(1_284);
        let head = log.lines().take(400).collect::<Vec<_>>().join("\n");
        let spec = RowSpec {
            title: "l1".to_string(),
            kind: Some("text".to_string()),
            preview: Some(Preview {
                text: head.clone(),
                mono: true,
            }),
            ..RowSpec::default()
        };
        let row = row(spec, Some(log.as_str()));
        assert_eq!(
            body_of(&row, false, Detail::Inline),
            Body::Text(whole_text(&head, true))
        );
        let Body::Text(excerpt) = body_of(&row, true, Detail::Inline) else {
            panic!("a log is text");
        };
        assert!(excerpt.numbered);
        let lines = head_and_tail(&excerpt, 24);
        assert_eq!(lines[17], Line::Hidden(1284 - 17 - 6));
        assert_eq!(
            lines[23],
            Line::Text {
                number: Some(1284),
                text: "l1284".to_string()
            }
        );
    }

    #[test]
    fn the_sheet_shows_head_divider_and_tail_with_real_numbers() {
        for excerpt in [whole_text(&log(1_284), true), log_excerpt(1_284, 40, 12)] {
            let lines = head_and_tail(&excerpt, 24);
            assert_eq!(lines.len(), 24);
            assert_eq!(
                lines[16],
                Line::Text {
                    number: Some(17),
                    text: "l17".to_string()
                }
            );
            assert_eq!(lines[17], Line::Hidden(1284 - 17 - 6));
            assert_eq!(
                lines[18],
                Line::Text {
                    number: Some(1279),
                    text: "l1279".to_string()
                }
            );
            assert_eq!(
                lines[23],
                Line::Text {
                    number: Some(1284),
                    text: "l1284".to_string()
                }
            );
        }
        let all_in_head = log_excerpt(30, 30, 0);
        let lines = head_and_tail(&all_in_head, 24);
        assert_eq!(lines[17], Line::Hidden(30 - 17 - 6));
        assert_eq!(
            lines[23],
            Line::Text {
                number: Some(30),
                text: "l30".to_string()
            }
        );
        assert_eq!(head_and_tail(&whole_text("a\nb", true), 24).len(), 2);
        assert_eq!(grouped(1261), "1,261");
        assert_eq!(grouped(12), "12");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn the_size_fact_says_how_much_the_frame_shows() {
        let fit = fits(322.0);
        let counted = |text: DetailText| Body::Text(text).counted().unwrap();
        let long = counted(log_excerpt(1_284, 40, 12));
        assert_eq!(shown(long, fit, true), " \u{b7} 17 shown");
        let code = counted(whole_text(&log(6), true));
        assert_eq!(shown(code, fit, false), ", all shown");
        let prose = counted(whole_text("a paragraph", false));
        assert_eq!(shown(prose, fit, true), " \u{b7} more below");
        assert_eq!(shown(prose, fit, false), ", all shown");
        assert_eq!(Body::Loading.counted(), None);
    }
}
