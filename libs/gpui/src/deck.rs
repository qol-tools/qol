use gpui::*;

use crate::theme::SettingsPanelPalette;

pub const TRANSITION: std::time::Duration = std::time::Duration::from_millis(180);
pub const CARD_ACCENT: f32 = 1.5;

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
}

pub fn front_offset(depth: usize) -> f32 {
    if depth > 1 {
        18.0
    } else {
        10.0
    }
}

pub fn slide(step: usize, motion: Option<Motion>, depth: usize, width: f32) -> Option<Slide> {
    let to = if depth == 0 { 0.0 } else { front_offset(depth) };
    let from = match motion {
        Some(Motion::Push) => width,
        Some(Motion::Pop) => front_offset(depth + 1),
        None => to,
    };
    (from != to).then_some(Slide { step, from, to })
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

fn card_edges(card: Div, palette: SettingsPanelPalette, hairline: Rgba) -> Div {
    card.bg(rgb(palette.window_bg))
        .border_t(px(1.))
        .border_r(px(1.))
        .border_b(px(1.))
        .border_color(hairline)
        .rounded_l(px(qol_theme::RADIUS_CARD))
}

pub fn render(
    palette: SettingsPanelPalette,
    depth: usize,
    card: Div,
    slide: Option<Slide>,
    animation_id: &'static str,
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
    .left(px(front_offset(depth)))
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
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .children(slivers_for(depth).into_iter().map(|sliver| {
            card_edges(
                div()
                    .absolute()
                    .left(px(sliver.left))
                    .w(px(sliver.width))
                    .top(px(sliver.inset))
                    .bottom(px(sliver.inset)),
                palette,
                hairline,
            )
            .child(accent())
        }))
        .child(front)
}

#[cfg(test)]
mod tests {
    use super::{front_offset, slide, slivers_for, Motion, Slide, Sliver};

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
    fn slide_maps_counter_and_direction_to_key_and_opposite_starts() {
        let push = slide(7, Some(Motion::Push), 1, 520.0).unwrap();
        let pop = slide(8, Some(Motion::Pop), 1, 520.0).unwrap();
        assert_ne!(push.step, pop.step);
        assert_eq!(push.step, 7);
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
            })
        );
        assert_eq!(slide(10, None, 2, 520.0), None);
        assert_eq!(front_offset(1), 10.0);
        assert_eq!(front_offset(2), 18.0);
    }
}
