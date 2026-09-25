use gpui::*;

use crate::theme::SettingsPanelPalette;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    Push,
    Pop,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Slide {
    pub step: usize,
    pub from: f32,
    pub to: f32,
    pub from_depth: usize,
}

pub type SliverClick = std::rc::Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub struct DeckFrame {
    pub depth: usize,
    pub slide: Option<Slide>,
    pub closing: Option<(Div, Slide)>,
    pub animation_id: &'static str,
    pub marks: Vec<Option<f32>>,
    pub on_sliver: Option<SliverClick>,
}

pub fn resting(depth: usize) -> f32 {
    if depth == 0 {
        0.0
    } else {
        9.0 * depth as f32 + 1.0
    }
}

pub fn edge_alpha(depth: usize, index: usize) -> f32 {
    (0.5 * 0.62_f32.powi(depth.saturating_sub(1).saturating_sub(index) as i32)).max(0.08)
}

pub fn rail_opacity(depth: usize) -> f32 {
    match depth {
        0 | 1 => 0.5,
        _ => (0.5 * 0.65_f32.powi(depth as i32 - 1)).max(0.12),
    }
}

pub fn exit(step: usize, depth: usize, width: f32) -> Slide {
    Slide {
        step,
        from: resting(depth),
        to: width,
        from_depth: depth,
    }
}

pub fn after_transition<V: 'static>(
    cx: &mut Context<V>,
    finish: impl FnOnce(&mut V, &mut Context<V>) + 'static,
) {
    cx.spawn(move |view: WeakEntity<V>, cx: &mut AsyncApp| {
        let mut async_cx = cx.clone();
        async move {
            async_cx
                .background_executor()
                .timer(qol_theme::Motion::SETTLE.duration)
                .await;
            let _ = view.update(&mut async_cx, finish);
        }
    })
    .detach();
}

pub fn slide(step: usize, motion: Option<Motion>, depth: usize, width: f32) -> Option<Slide> {
    let to = resting(depth);
    let (from, from_depth) = match motion {
        Some(Motion::Push) => (width, depth.saturating_sub(1)),
        Some(Motion::Pop) => (resting(depth + 1), depth + 1),
        None => (to, depth),
    };
    (from != to).then_some(Slide {
        step,
        from,
        to,
        from_depth,
    })
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Sliver {
    left: f32,
    width: f32,
    inset: f32,
}

fn slivers_for(depth: usize) -> Vec<Sliver> {
    (0..depth)
        .map(|index| Sliver {
            left: 9.0 * index as f32,
            width: 12.0,
            inset: 7.0 * (depth - index) as f32,
        })
        .collect()
}

fn sliver_start(from_depth: usize, index: usize, target: Sliver) -> Sliver {
    match slivers_for(from_depth).get(index) {
        Some(sliver) => *sliver,
        None => Sliver {
            left: resting(from_depth),
            width: target.width,
            inset: 0.0,
        },
    }
}

fn step_between(start: f32, end: f32, delta: f32) -> f32 {
    start + (end - start) * delta
}

fn with_alpha(color: u32, opacity: f32) -> Rgba {
    let mut color = rgb(color);
    color.a = opacity;
    color
}

fn blend(start: Rgba, end: Rgba, delta: f32) -> Rgba {
    Rgba {
        r: start.r + (end.r - start.r) * delta,
        g: start.g + (end.g - start.g) * delta,
        b: start.b + (end.b - start.b) * delta,
        a: start.a + (end.a - start.a) * delta,
    }
}

fn stage() -> Div {
    div().relative().flex_1().min_w_0().h_full()
}

pub(crate) fn card_edges(card: Div, palette: SettingsPanelPalette, hairline: Rgba) -> Div {
    card.bg(rgb(palette.window_bg))
        .border(px(1.))
        .border_color(hairline)
        .rounded_l(px(qol_theme::RADIUS_CARD))
        .occlude()
}

/// The card that is on its way out. It keeps its own content while it slides
/// off to the right, so the page underneath is revealed instead of replaced.
pub fn drawer(palette: SettingsPanelPalette, card: Div, slide: Slide) -> AnyElement {
    let hairline = rgba(crate::kit::kit().washes.hairline.packed());
    card_edges(
        card.absolute().right_0().top_0().bottom_0(),
        palette,
        hairline,
    )
    .shadow(crate::kit::float_shadow(palette.section_text))
    .with_animation(
        ("settings-card-drawer", slide.step),
        crate::motion::animation(qol_theme::Motion::SETTLE),
        move |card, delta| card.left(px(slide.from + (slide.to - slide.from) * delta)),
    )
    .into_any_element()
}

pub fn reveal(palette: SettingsPanelPalette, page: Div, card: Div, slide: Slide) -> Div {
    stage()
        .child(page.absolute().inset_0())
        .child(drawer(palette, card, slide))
}

fn animate_sliver<E: Styled + IntoElement + 'static>(
    stub: E,
    animation_id: &'static str,
    index: usize,
    shrink: Slide,
    start: Sliver,
    target: Sliver,
) -> AnyElement {
    stub.with_animation(
        (
            SharedString::from(format!("{animation_id}-sliver-{index}")),
            shrink.step,
        ),
        crate::motion::animation(qol_theme::Motion::SETTLE),
        move |stub, delta| {
            stub.left(px(step_between(start.left, target.left, delta)))
                .w(px(step_between(start.width, target.width, delta)))
                .top(px(step_between(start.inset, target.inset, delta)))
                .bottom(px(step_between(start.inset, target.inset, delta)))
        },
    )
    .into_any_element()
}

pub fn render(palette: SettingsPanelPalette, card: Div, frame: DeckFrame) -> Div {
    let hairline = rgba(crate::kit::kit().washes.hairline.packed());
    let DeckFrame {
        depth,
        slide,
        closing,
        animation_id,
        marks,
        on_sliver,
    } = frame;
    let front = card_edges(
        card.absolute().right_0().top_0().bottom_0(),
        palette,
        hairline,
    )
    .left(px(resting(depth)))
    .shadow(crate::kit::float_shadow(palette.section_text));
    let front = match slide {
        Some(slide) => front
            .with_animation(
                (animation_id, slide.step),
                crate::motion::animation(qol_theme::Motion::SETTLE),
                move |card, delta| card.left(px(slide.from + (slide.to - slide.from) * delta)),
            )
            .into_any_element(),
        None => front.into_any_element(),
    };
    let shrink = slide
        .or_else(|| closing.as_ref().map(|(_, slide)| *slide))
        .filter(|slide| slide.from_depth != depth);
    let leaving = closing.map(|(card, slide)| drawer(palette, card, slide));
    stage()
        .children(
            slivers_for(depth)
                .into_iter()
                .enumerate()
                .map(|(index, sliver)| {
                    let born = shrink
                        .map(|shrink| slivers_for(shrink.from_depth).get(index).is_none())
                        .unwrap_or(false);
                    let start = shrink.map(|shrink| sliver_start(shrink.from_depth, index, sliver));
                    let mark = marks.get(index).copied().flatten().map(|y| {
                        let mark_end =
                            with_alpha(palette.status_muted, 0.9 * edge_alpha(depth, index));
                        let base = div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(y - 14.0 - sliver.inset))
                            .h(px(qol_theme::HEIGHT_INLINE));
                        match shrink {
                            Some(shrink) => {
                                let mark_start = with_alpha(
                                    palette.status_muted,
                                    if born {
                                        0.0
                                    } else {
                                        0.9 * edge_alpha(shrink.from_depth, index)
                                    },
                                );
                                base.bg(mark_end)
                                    .with_animation(
                                        (
                                            SharedString::from(format!(
                                                "{animation_id}-sliver-{index}-mark"
                                            )),
                                            shrink.step,
                                        ),
                                        crate::motion::animation(qol_theme::Motion::SETTLE),
                                        move |mark, delta| {
                                            mark.bg(blend(mark_start, mark_end, delta))
                                        },
                                    )
                                    .into_any_element()
                            }
                            None => base.bg(mark_end).into_any_element(),
                        }
                    });
                    let stub = card_edges(
                        div()
                            .absolute()
                            .left(px(sliver.left))
                            .w(px(sliver.width))
                            .top(px(sliver.inset))
                            .bottom(px(sliver.inset)),
                        palette,
                        hairline,
                    )
                    .overflow_hidden()
                    .children(mark);
                    if let Some(on_sliver) = on_sliver.as_ref() {
                        let on_sliver = on_sliver.clone();
                        let stub = stub
                            .id(("settings-deck-sliver", index))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                on_sliver(index, window, cx)
                            });
                        match (shrink, start) {
                            (Some(shrink), Some(start)) => {
                                animate_sliver(stub, animation_id, index, shrink, start, sliver)
                            }
                            _ => stub.into_any_element(),
                        }
                    } else {
                        match (shrink, start) {
                            (Some(shrink), Some(start)) => {
                                animate_sliver(stub, animation_id, index, shrink, start, sliver)
                            }
                            _ => stub.into_any_element(),
                        }
                    }
                }),
        )
        .child(front)
        .children(leaving)
}

#[cfg(test)]
mod tests {
    use super::{
        edge_alpha, exit, rail_opacity, resting, slide, sliver_start, slivers_for, Motion, Slide,
        Sliver,
    };

    #[test]
    fn slivers_follow_the_card_step_geometry() {
        assert_eq!(slivers_for(0), Vec::new());
        assert_eq!(
            slivers_for(1),
            vec![Sliver {
                left: 0.0,
                width: 12.0,
                inset: 7.0,
            }]
        );
        assert_eq!(
            slivers_for(2),
            vec![
                Sliver {
                    left: 0.0,
                    width: 12.0,
                    inset: 14.0,
                },
                Sliver {
                    left: 9.0,
                    width: 12.0,
                    inset: 7.0,
                },
            ]
        );
        assert_eq!(
            slivers_for(3),
            vec![
                Sliver {
                    left: 0.0,
                    width: 12.0,
                    inset: 21.0,
                },
                Sliver {
                    left: 9.0,
                    width: 12.0,
                    inset: 14.0,
                },
                Sliver {
                    left: 18.0,
                    width: 12.0,
                    inset: 7.0,
                },
            ]
        );
        let depth_five = slivers_for(5);
        assert_eq!(depth_five.len(), 5);
        assert_eq!(depth_five[0].inset, 35.0);
        assert_eq!(depth_five[4].left, 36.0);
    }

    #[test]
    fn a_sliver_grows_out_of_the_card_it_came_from() {
        let target = slivers_for(1)[0];
        assert_eq!(
            sliver_start(0, 0, target),
            Sliver {
                left: 0.0,
                width: target.width,
                inset: 0.0,
            }
        );
        assert_eq!(sliver_start(1, 0, slivers_for(2)[0]), slivers_for(1)[0]);
        assert_eq!(
            sliver_start(1, 1, slivers_for(2)[1]),
            Sliver {
                left: 10.0,
                width: slivers_for(2)[1].width,
                inset: 0.0,
            }
        );
    }

    #[test]
    fn a_leaving_card_starts_where_it_rests_and_ends_off_the_body() {
        assert_eq!(
            exit(4, 1, 520.0),
            Slide {
                step: 4,
                from: 10.0,
                to: 520.0,
                from_depth: 1,
            }
        );
        assert_eq!(exit(5, 0, 520.0).from, 0.0);
        assert_eq!(exit(6, 2, 520.0).from, 19.0);
    }

    #[test]
    fn slide_maps_counter_and_direction_to_key_and_opposite_starts() {
        let push = slide(7, Some(Motion::Push), 1, 520.0).unwrap();
        let pop = slide(8, Some(Motion::Pop), 1, 520.0).unwrap();
        assert_ne!(push.step, pop.step);
        assert_eq!(push.step, 7);
        assert_eq!(push.from_depth, 0);
        assert_eq!(pop.from_depth, 2);
        assert_eq!(pop.step, 8);
        assert_eq!(push.from, 520.0);
        assert_eq!(pop.from, 19.0);
        assert!(push.from > pop.from);
        assert_eq!(push.to, pop.to);
        assert_eq!(
            slide(9, Some(Motion::Pop), 0, 520.0),
            Some(Slide {
                step: 9,
                from: 10.0,
                to: 0.0,
                from_depth: 1,
            })
        );
        assert_eq!(slide(10, None, 2, 520.0), None);
        assert_eq!(resting(0), 0.0);
        assert_eq!(resting(1), 10.0);
        assert_eq!(resting(2), 19.0);
        assert_eq!(resting(3), 28.0);
    }

    #[test]
    fn edge_alpha_fades_steeply_to_a_floor() {
        assert_eq!(edge_alpha(1, 0), 0.5);
        assert!((edge_alpha(2, 0) - 0.31).abs() < 1e-6);
        assert!((edge_alpha(3, 0) - 0.1922).abs() < 1e-4);
        assert_eq!(edge_alpha(5, 0), 0.08);
    }

    #[test]
    fn rail_opacity_follows_the_ramp() {
        assert_eq!(rail_opacity(0), 0.5);
        assert_eq!(rail_opacity(1), 0.5);
        assert!((rail_opacity(2) - 0.325).abs() < 1e-6);
        assert!((rail_opacity(3) - 0.21125).abs() < 1e-6);
        assert!((rail_opacity(4) - 0.1373).abs() < 1e-4);
        assert_eq!(rail_opacity(5), 0.12);
    }
}
