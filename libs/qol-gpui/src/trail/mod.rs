use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, Animation, AnimationExt as _, AnyElement, App, Div, ElementId, Hsla, RenderOnce,
    Window,
};
use qol_theme::SystemPalette;

pub mod model;
pub mod motion;

pub use model::TrailItem;

const PAD_X: f32 = 14.0;
const DOT_SIZE: f32 = 11.0;
const DOT_LIT_SIZE: f32 = 14.0;
const GLOW_PAD: f32 = 4.0;
const HEAD_SIZE: f32 = 7.0;
const LINE_WIDTH: f32 = 1.5;
const LINE_CX: f32 = PAD_X + 6.0;
const TEXT_LEFT: f32 = PAD_X + 24.0;
const META_LINE_HEIGHT: f32 = 15.0;
const BODY_LINE_HEIGHT: f32 = 18.0;
const META_GAP: f32 = 8.0;
const META_BODY_GAP: f32 = 2.0;

#[derive(IntoElement)]
pub struct Trail {
    id: ElementId,
    items: Vec<TrailItem>,
    from: f32,
    from_index: usize,
    to: usize,
    seq: u64,
    settled: bool,
    palette: SystemPalette,
}

impl Trail {
    pub fn new(id: impl Into<ElementId>, items: Vec<TrailItem>) -> Self {
        Self {
            id: id.into(),
            items,
            from: 0.0,
            from_index: 0,
            to: 0,
            seq: 0,
            settled: false,
            palette: qol_theme::runtime_theme().system,
        }
    }

    pub fn focus(mut self, from: f32, from_index: usize, to: usize) -> Self {
        self.from = from;
        self.from_index = from_index;
        self.to = to;
        self
    }

    pub fn seq(mut self, seq: u64) -> Self {
        self.seq = seq;
        self
    }

    pub fn settled(mut self, settled: bool) -> Self {
        self.settled = settled;
        self
    }

    pub fn palette(mut self, palette: SystemPalette) -> Self {
        self.palette = palette;
        self
    }
}

impl RenderOnce for Trail {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        if self.items.is_empty() {
            return div()
                .h(px(motion::viewport_height()))
                .overflow_hidden()
                .into_any_element();
        }

        let len = self.items.len();
        let items = self.items;
        let node_ids: Vec<ElementId> = (0..len)
            .map(|index| (self.id.clone(), index.to_string()).into())
            .collect();
        let from = self.from;
        let from_index = self.from_index;
        let to = self.to;
        let palette = self.palette;
        let viewport = || div().h(px(motion::viewport_height())).overflow_hidden();
        let inner = || {
            div()
                .absolute()
                .left_0()
                .right_0()
                .pt(px(motion::PAD_TOP))
                .flex()
                .flex_col()
        };

        if self.settled {
            return viewport()
                .child(track(
                    inner(),
                    Frame {
                        items: &items,
                        node_ids: &node_ids,
                        from,
                        from_index,
                        to,
                        len,
                        phase: motion::Phase::Drain,
                        delta: 1.0,
                        palette,
                    },
                ))
                .into_any_element();
        }

        let animation_id: ElementId = (self.id.clone(), self.seq.to_string()).into();
        viewport()
            .child(inner().with_animations(
                animation_id,
                vec![
                    Animation::new(Duration::from_millis(motion::TRAVEL_MS))
                        .with_easing(motion::ease_travel),
                    Animation::new(Duration::from_millis(motion::DRAIN_MS))
                        .with_easing(motion::ease_drain),
                ],
                move |inner, ix, delta| {
                    let phase = if ix == 0 {
                        motion::Phase::Travel
                    } else {
                        motion::Phase::Drain
                    };
                    track(
                        inner,
                        Frame {
                            items: &items,
                            node_ids: &node_ids,
                            from,
                            from_index,
                            to,
                            len,
                            phase,
                            delta,
                            palette,
                        },
                    )
                },
            ))
            .into_any_element()
    }
}

struct Frame<'a> {
    items: &'a [TrailItem],
    node_ids: &'a [ElementId],
    from: f32,
    from_index: usize,
    to: usize,
    len: usize,
    phase: motion::Phase,
    delta: f32,
    palette: SystemPalette,
}

fn track(track: Div, frame: Frame<'_>) -> Div {
    let Frame {
        items,
        node_ids,
        from,
        from_index,
        to,
        len,
        phase,
        delta,
        palette,
    } = frame;
    let (segment_top, segment_height) = motion::segment(from, to, phase, delta);
    let lit = motion::lit(to, phase);
    let here = motion::here(from_index, to, phase);
    let fill = motion::fill(phase, delta);
    let mut track = track
        .top(px(motion::slide(from, to, len, phase, delta)))
        .child(spine(palette, len))
        .child(
            div()
                .absolute()
                .left(px(LINE_CX - LINE_WIDTH / 2.0))
                .w(px(LINE_WIDTH))
                .top(px(segment_top))
                .h(px(segment_height))
                .bg(rgb(palette.accent)),
        );
    if let Some(center) = motion::head_center(from, to, phase, delta) {
        track = track.child(
            div()
                .absolute()
                .left(px(LINE_CX - HEAD_SIZE / 2.0))
                .top(px(center - HEAD_SIZE / 2.0))
                .size(px(HEAD_SIZE))
                .rounded_full()
                .bg(rgb(palette.accent)),
        );
    }
    track.children(
        node_ids
            .iter()
            .zip(items.iter())
            .enumerate()
            .map(|(index, (node_id, item))| {
                node(node_id.clone(), item, index, lit, here, fill, palette)
            }),
    )
}

fn spine(palette: SystemPalette, len: usize) -> AnyElement {
    div()
        .absolute()
        .left(px(LINE_CX - LINE_WIDTH / 2.0))
        .w(px(LINE_WIDTH))
        .top(px(motion::dot_center(0.0)))
        .h(px(
            motion::dot_center(len as f32 - 1.0) - motion::dot_center(0.0)
        ))
        .bg(rgb(palette.border_subtle))
        .into_any_element()
}

fn node(
    id: ElementId,
    item: &TrailItem,
    index: usize,
    lit: Option<usize>,
    here: usize,
    fill: f32,
    palette: SystemPalette,
) -> AnyElement {
    let is_here = index == here;
    let body_tone = if item.struck {
        palette.text_muted
    } else if is_here {
        palette.text_primary
    } else if index < here {
        palette.text_secondary
    } else {
        palette.text_muted
    };
    let mut body = div()
        .text_size(px(qol_theme::TEXT_MICRO))
        .line_height(px(BODY_LINE_HEIGHT))
        .text_color(rgb(body_tone))
        .line_clamp(3)
        .child(item.text.clone());
    if index > here && !item.struck {
        body = body.opacity(0.72);
    }
    if item.struck {
        body = body.line_through();
    }
    let (at_tone, tag_tone) = if is_here {
        (palette.text_secondary, palette.accent_ink)
    } else {
        (palette.text_muted, palette.text_muted)
    };
    let mut children = Vec::new();
    if lit == Some(index) {
        let size = DOT_SIZE + (DOT_LIT_SIZE - DOT_SIZE) * fill;
        let glow_size = size + 2.0 * GLOW_PAD;
        let glow: Hsla = rgb(palette.accent).into();
        children.push(
            div()
                .absolute()
                .left(px(LINE_CX - glow_size / 2.0))
                .top(px(11.5 - glow_size / 2.0))
                .size(px(glow_size))
                .rounded_full()
                .bg(glow.opacity(0.22 * fill))
                .into_any_element(),
        );
        children.push(
            div()
                .absolute()
                .left(px(LINE_CX - size / 2.0))
                .top(px(11.5 - size / 2.0))
                .size(px(size))
                .rounded_full()
                .bg(rgb(palette.accent))
                .into_any_element(),
        );
    } else {
        children.push(
            div()
                .absolute()
                .left(px(LINE_CX - DOT_SIZE / 2.0))
                .top(px(11.5 - DOT_SIZE / 2.0))
                .size(px(DOT_SIZE))
                .rounded_full()
                .bg(rgb(palette.surface_elevated))
                .border(px(1.5))
                .border_color(rgb(palette.border_subtle))
                .into_any_element(),
        );
    }
    children.push(
        div()
            .absolute()
            .left(px(TEXT_LEFT))
            .right(px(PAD_X))
            .top_0()
            .flex()
            .flex_col()
            .gap(px(META_BODY_GAP))
            .child(
                div()
                    .flex()
                    .gap(px(META_GAP))
                    .text_size(px(qol_theme::TEXT_NANO))
                    .line_height(px(META_LINE_HEIGHT))
                    .child(div().text_color(rgb(at_tone)).child(item.at.clone()))
                    .child(
                        div()
                            .text_color(rgb(tag_tone))
                            .child(item.tag.to_uppercase()),
                    ),
            )
            .child(body)
            .into_any_element(),
    );
    div()
        .relative()
        .w_full()
        .h(px(motion::ROW_H))
        .overflow_hidden()
        .children(children)
        .id(id)
        .into_any_element()
}
