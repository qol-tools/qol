use std::f32::consts::TAU;

use gpui::prelude::*;
use gpui::{canvas, div, point, px, rgba, App, Bounds, ElementId, Pixels, RenderOnce, Window};
use qol_theme::{translucent, Alpha};

const RING_LINE: f32 = 1.5;
const INNER_RING_SHARE: f32 = 0.46;
const INNER_RING_FLOOR: f32 = 24.0;
const CORE_SHARE: f32 = 0.16;
const CORE_FLOOR: f32 = 4.0;
const SWEEP_TRAIL: f32 = 0.3;
const SWEEP_PEAK: f32 = 0.55;
const SWEEP_WEDGES: usize = 18;
const CIRCLE_POINTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkState {
    Live,
    Rest,
    Off,
}

#[derive(IntoElement)]
pub struct SearchMark {
    id: ElementId,
    size: f32,
    state: MarkState,
    live: u32,
    rest: u32,
}

impl SearchMark {
    pub fn new(
        id: impl Into<ElementId>,
        size: f32,
        state: MarkState,
        live: u32,
        rest: u32,
    ) -> Self {
        Self {
            id: id.into(),
            size,
            state,
            live,
            rest,
        }
    }
}

impl RenderOnce for SearchMark {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let live = self.state == MarkState::Live;
        let turn = crate::activity_animation::progress_of(qol_theme::MOTION_LOOP * 2);
        let tone = if live { self.live } else { self.rest };
        let size = self.size;
        let drawing = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                paint_mark(bounds, window, size, tone, live.then_some(turn))
            },
        )
        .size(px(size));
        let mark = div().flex_none().size(px(size)).child(drawing);
        let mark = if self.state == MarkState::Off {
            mark.opacity(qol_theme::OPACITY_FILL_HELD)
        } else {
            mark
        };
        crate::activity_animation::ActivityAnimation::new(self.id, live, mark)
    }
}

fn paint_mark(
    bounds: Bounds<Pixels>,
    window: &mut Window,
    size: f32,
    tone: u32,
    sweep: Option<f32>,
) {
    let center = bounds.center();
    let at = |radius: f32, turn: f32| {
        let angle = turn * TAU;
        point(
            center.x + px(radius * angle.sin()),
            center.y - px(radius * angle.cos()),
        )
    };
    let outer = size / 2.0;
    if let Some(turn) = sweep {
        for (start, end, strength) in sweep_wedges(turn) {
            let steps = 3;
            let mut points = vec![center];
            points.extend(
                (0..=steps)
                    .map(|step| at(outer, start + (end - start) * step as f32 / steps as f32)),
            );
            let mut path = gpui::PathBuilder::fill();
            path.add_polygon(&points, true);
            if let Ok(path) = path.build() {
                window.paint_path(path, crate::deck::with_alpha(tone, strength));
            }
        }
    }
    let circle = |radius: f32| {
        (0..CIRCLE_POINTS)
            .map(|place| at(radius, place as f32 / CIRCLE_POINTS as f32))
            .collect::<Vec<_>>()
    };
    let mut ring = gpui::PathBuilder::stroke(px(RING_LINE));
    ring.add_polygon(&circle(outer - RING_LINE / 2.0), true);
    if let Ok(path) = ring.build() {
        window.paint_path(path, rgba(translucent(tone, Alpha::Veil)));
    }
    if size > INNER_RING_FLOOR {
        let mut inner = gpui::PathBuilder::stroke(px(RING_LINE));
        inner.add_polygon(
            &circle(size * INNER_RING_SHARE / 2.0 - RING_LINE / 2.0),
            true,
        );
        if let Ok(path) = inner.build() {
            window.paint_path(path, rgba(translucent(tone, Alpha::Strong)));
        }
    }
    let mut core = gpui::PathBuilder::fill();
    core.add_polygon(&circle((size * CORE_SHARE).max(CORE_FLOOR) / 2.0), true);
    if let Ok(path) = core.build() {
        window.paint_path(path, gpui::rgb(tone));
    }
}

fn sweep_wedges(turn: f32) -> Vec<(f32, f32, f32)> {
    let width = SWEEP_TRAIL / SWEEP_WEDGES as f32;
    (0..SWEEP_WEDGES)
        .map(|wedge| {
            let start = turn - SWEEP_TRAIL + width * wedge as f32;
            let strength = SWEEP_PEAK * (wedge as f32 + 0.5) / SWEEP_WEDGES as f32;
            (start, start + width, strength)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{sweep_wedges, SWEEP_PEAK, SWEEP_TRAIL, SWEEP_WEDGES};

    #[test]
    fn the_sweep_brightens_toward_its_leading_edge() {
        let wedges = sweep_wedges(0.25);
        assert_eq!(wedges.len(), SWEEP_WEDGES);
        let (first, _, faint) = wedges[0];
        let (_, last, bright) = wedges[SWEEP_WEDGES - 1];
        assert!((first - (0.25 - SWEEP_TRAIL)).abs() < 1e-6);
        assert!((last - 0.25).abs() < 1e-6);
        assert!(faint < bright && bright < SWEEP_PEAK);
        assert!(wedges.windows(2).all(|pair| pair[0].2 < pair[1].2));
    }
}
