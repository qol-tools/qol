use gpui::prelude::*;
use gpui::{div, px, App, ElementId, Hsla, Pixels, RenderOnce, SharedString, Window};

const DOTS: usize = 8;
const DEFAULT_SIZE: Pixels = px(14.);
const DOT_SHARE: f32 = 3. / 14.;
const TAIL_OPACITY: f32 = 0.25;

#[derive(IntoElement)]
pub struct Busy {
    id: ElementId,
    label: Option<SharedString>,
    color: Hsla,
    size: Pixels,
}

impl Busy {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        color: impl Into<Hsla>,
    ) -> Self {
        Self {
            label: Some(label.into()),
            ..Self::ring(id, color)
        }
    }

    pub fn ring(id: impl Into<ElementId>, color: impl Into<Hsla>) -> Self {
        Self {
            id: id.into(),
            label: None,
            color: color.into(),
            size: DEFAULT_SIZE,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for Busy {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let step = step_of(crate::activity_animation::progress_of(
            qol_theme::MOTION_LOOP,
        ));
        let size = f32::from(self.size);
        let dot = size * DOT_SHARE;
        let radius = (size - dot) / 2.;
        let ring = div()
            .relative()
            .flex_none()
            .size(self.size)
            .children((0..DOTS).map(|place| {
                let angle = std::f32::consts::TAU * place as f32 / DOTS as f32;
                div()
                    .absolute()
                    .left(px(radius + radius * angle.sin()))
                    .top(px(radius - radius * angle.cos()))
                    .size(px(dot))
                    .rounded_full()
                    .bg(self.color)
                    .opacity(dot_opacity(place, step))
            }));
        let ring = crate::activity_animation::ActivityAnimation::new(self.id, true, ring)
            .interval(qol_theme::MOTION_LOOP / DOTS as u32);
        match self.label {
            None => ring.into_any_element(),
            Some(label) => div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(qol_theme::SPACE_INSET))
                .text_color(self.color)
                .child(ring)
                .child(label)
                .into_any_element(),
        }
    }
}

fn step_of(progress: f32) -> usize {
    (progress * DOTS as f32) as usize % DOTS
}

fn dot_opacity(place: usize, step: usize) -> f32 {
    let age = (place + DOTS - step) % DOTS;
    TAIL_OPACITY + (1. - TAIL_OPACITY) * age as f32 / (DOTS - 1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_progress_maps_to_one_step_per_dot() {
        let cases = [
            (0., 0),
            (0.124, 0),
            (0.125, 1),
            (0.5, 4),
            (0.999, 7),
            (1., 0),
        ];
        for (progress, expected) in cases {
            assert_eq!(step_of(progress), expected);
        }
    }

    #[test]
    fn the_brightest_dot_leads_clockwise_with_the_faintest_just_ahead() {
        for step in 0..DOTS {
            let head = (step + DOTS - 1) % DOTS;
            assert_eq!(dot_opacity(head, step), 1.);
            assert_eq!(dot_opacity((head + 1) % DOTS, step), TAIL_OPACITY);
        }
    }
}
