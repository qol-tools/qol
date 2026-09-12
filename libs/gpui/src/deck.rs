use gpui::*;

use crate::theme::SettingsPanelPalette;

pub const TRANSITION: std::time::Duration = std::time::Duration::from_millis(180);
pub const CARD_ACCENT: f32 = qol_theme::SPACE_MARK;

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

pub fn front_offset(depth: usize) -> f32 {
    if depth > 1 {
        18.0
    } else {
        10.0
    }
}

pub fn resting(depth: usize) -> f32 {
    if depth == 0 {
        0.0
    } else {
        front_offset(depth)
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
            async_cx.background_executor().timer(TRANSITION).await;
            let _ = view.update(&mut async_cx, finish);
        }
    })
    .detach();
}

pub fn slide(step: usize, motion: Option<Motion>, depth: usize, width: f32) -> Option<Slide> {
    let to = resting(depth);
    let (from, from_depth) = match motion {
        Some(Motion::Push) => (width, depth.saturating_sub(1)),
        Some(Motion::Pop) => (front_offset(depth + 1), depth + 1),
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
    match depth {
        0 => Vec::new(),
        1 => vec![Sliver {
            left: 0.0,
            width: 12.0,
            inset: 8.0,
        }],
        _ => vec![
            Sliver {
                left: 0.0,
                width: 10.0,
                inset: 14.0,
            },
            Sliver {
                left: 9.0,
                width: 10.0,
                inset: 7.0,
            },
        ],
    }
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

fn stage() -> Div {
    div().relative().flex_1().min_w_0().h_full()
}

fn card_edges(card: Div, palette: SettingsPanelPalette, hairline: Rgba) -> Div {
    card.bg(rgb(palette.window_bg))
        .border_t(px(1.))
        .border_r(px(1.))
        .border_b(px(1.))
        .border_color(hairline)
        .rounded_l(px(qol_theme::RADIUS_CARD))
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
    .child(crate::kit::accent_left_edge(
        qol_theme::RADIUS_CARD,
        CARD_ACCENT,
        palette.row_border_selected,
    ))
    .with_animation(
        ("settings-card-drawer", slide.step),
        Animation::new(TRANSITION).with_easing(ease_out_quint()),
        move |card, delta| card.left(px(slide.from + (slide.to - slide.from) * delta)),
    )
    .into_any_element()
}

pub fn reveal(palette: SettingsPanelPalette, page: Div, card: Div, slide: Slide) -> Div {
    stage()
        .child(page.absolute().inset_0())
        .child(drawer(palette, card, slide))
}

pub fn render(
    palette: SettingsPanelPalette,
    depth: usize,
    card: Div,
    slide: Option<Slide>,
    animation_id: &'static str,
    closing: Option<(Div, Slide)>,
) -> Div {
    let hairline = rgba(crate::kit::kit().washes.hairline.packed());
    let accent = || {
        crate::kit::accent_left_edge(
            qol_theme::RADIUS_CARD,
            CARD_ACCENT,
            palette.row_border_selected,
        )
    };
    let front = card_edges(
        card.absolute().right_0().top_0().bottom_0(),
        palette,
        hairline,
    )
    .left(px(resting(depth)))
    .shadow(crate::kit::float_shadow(palette.section_text))
    .child(accent());
    let front = match slide {
        Some(slide) => front
            .with_animation(
                (animation_id, slide.step),
                Animation::new(TRANSITION).with_easing(ease_out_quint()),
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
                    .child(accent());
                    let Some(shrink) = shrink else {
                        return stub.into_any_element();
                    };
                    let start = sliver_start(shrink.from_depth, index, sliver);
                    stub.with_animation(
                        (
                            SharedString::from(format!("{animation_id}-sliver-{index}")),
                            shrink.step,
                        ),
                        Animation::new(TRANSITION).with_easing(ease_out_quint()),
                        move |stub, delta| {
                            stub.left(px(step_between(start.left, sliver.left, delta)))
                                .w(px(step_between(start.width, sliver.width, delta)))
                                .top(px(step_between(start.inset, sliver.inset, delta)))
                                .bottom(px(step_between(start.inset, sliver.inset, delta)))
                        },
                    )
                    .into_any_element()
                }),
        )
        .child(front)
        .children(leaving)
}

#[cfg(test)]
mod tests {
    use super::{exit, front_offset, slide, sliver_start, slivers_for, Motion, Slide, Sliver};

    #[test]
    fn slivers_follow_the_one_window_depth_geometry() {
        assert_eq!(slivers_for(0), Vec::new());
        assert_eq!(
            slivers_for(1),
            vec![Sliver {
                left: 0.0,
                width: 12.0,
                inset: 8.0
            }]
        );
        let depth_two = slivers_for(2);
        assert_eq!(
            depth_two,
            vec![
                Sliver {
                    left: 0.0,
                    width: 10.0,
                    inset: 14.0
                },
                Sliver {
                    left: 9.0,
                    width: 10.0,
                    inset: 7.0
                },
            ]
        );
        assert_eq!(slivers_for(3), depth_two);
        assert_eq!(slivers_for(7), depth_two);
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
        assert_eq!(pop.from, 18.0);
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
        assert_eq!(front_offset(1), 10.0);
        assert_eq!(front_offset(2), 18.0);
    }
}
