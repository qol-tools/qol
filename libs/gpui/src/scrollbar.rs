use gpui::*;

pub const SEAM_TRACK_WIDTH: f32 = 3.0;
pub const SEAM_TRACK_INSET: f32 = 2.0;
pub const SEAM_THUMB_MIN: f32 = 24.0;

pub const OVERFLOW_FADE_HEIGHT: f32 = qol_theme::HEIGHT_HINT_BAR;
pub const OVERFLOW_STREAK_WIDTH: f32 = 180.0;
pub const OVERFLOW_STREAK_HEIGHT: f32 = qol_theme::HEIGHT_INLINE - qol_theme::SPACE_TIGHT;
pub const OVERFLOW_CUE_CENTRE: f32 = OVERFLOW_FADE_HEIGHT / 2.0;
pub const OVERFLOW_CHEVRON_WIDTH: f32 = qol_theme::SPACE_PAD;
pub const OVERFLOW_CHEVRON_RISE: f32 = qol_theme::SPACE_SNUG;
pub const OVERFLOW_CHEVRON_STROKE: f32 = 2.0;

pub fn seam_track(handle: ScrollHandle, track_rgba: u32, thumb_rgba: u32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let max = handle.max_offset().height;
            if max <= px(0.) {
                return;
            }
            let view_h = bounds.size.height;
            let track_x = bounds.right() - px(SEAM_TRACK_INSET + SEAM_TRACK_WIDTH);
            window.paint_quad(fill(
                Bounds::new(
                    point(track_x, bounds.top()),
                    size(px(SEAM_TRACK_WIDTH), view_h),
                ),
                rgba(track_rgba),
            ));
            let content = view_h + max;
            let thumb_h = (view_h * (view_h / content)).max(px(SEAM_THUMB_MIN));
            let frac = (-handle.offset().y / max).clamp(0.0, 1.0);
            let thumb_y = bounds.top() + (view_h - thumb_h) * frac;
            window.paint_quad(fill(
                Bounds::new(point(track_x, thumb_y), size(px(SEAM_TRACK_WIDTH), thumb_h)),
                rgba(thumb_rgba),
            ));
        },
    )
    .absolute()
    .inset_0()
}

enum OverflowEdge {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug)]
pub struct OverflowFadeStyle {
    pub surface_rgb: u32,
    pub ink_rgba: u32,
    pub wash_rgba: u32,
}

pub fn overflow_edges(
    viewport: Bounds<Pixels>,
    first: Bounds<Pixels>,
    last: Bounds<Pixels>,
    offset: Pixels,
) -> (bool, bool) {
    let top = first.top() + offset < viewport.top() - px(1.);
    let bottom = last.bottom() + offset > viewport.bottom() + px(1.);
    (top, bottom)
}

pub fn overflow_fade(
    handle: ScrollHandle,
    children: usize,
    style: OverflowFadeStyle,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let max = handle.max_offset().height;
            if max <= px(0.) {
                return;
            }
            if children == 0 {
                return;
            }
            let (Some(first), Some(last)) = (
                handle.bounds_for_item(0),
                handle.bounds_for_item(children - 1),
            ) else {
                return;
            };
            let (top, bottom) = overflow_edges(handle.bounds(), first, last, handle.offset().y);
            if top {
                paint_overflow_edge(window, bounds, OverflowEdge::Top, style);
            }
            if bottom {
                paint_overflow_edge(window, bounds, OverflowEdge::Bottom, style);
            }
        },
    )
    .absolute()
    .inset_0()
}

fn paint_overflow_edge(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    edge: OverflowEdge,
    style: OverflowFadeStyle,
) {
    let fade_h = px(OVERFLOW_FADE_HEIGHT).min(bounds.size.height);
    let rise = px(OVERFLOW_CHEVRON_RISE);
    let half_chevron = px(OVERFLOW_CHEVRON_WIDTH / 2.0);
    let (band_top, centre_y, tip_y, wing_y, band_start, band_end) = match edge {
        OverflowEdge::Top => {
            let centre_y = bounds.top() + px(OVERFLOW_CUE_CENTRE);
            (
                bounds.top(),
                centre_y,
                centre_y - rise * 0.5,
                centre_y + rise * 0.5,
                0xff,
                0x00,
            )
        }
        OverflowEdge::Bottom => {
            let centre_y = bounds.bottom() - px(OVERFLOW_CUE_CENTRE);
            (
                bounds.bottom() - fade_h,
                centre_y,
                centre_y + rise * 0.5,
                centre_y - rise * 0.5,
                0x00,
                0xff,
            )
        }
    };
    window.paint_quad(fill(
        Bounds::new(
            point(bounds.left(), band_top),
            size(bounds.size.width, fade_h),
        ),
        linear_gradient(
            180.0,
            linear_color_stop(rgba(crate::kit::alpha(style.surface_rgb, band_start)), 0.0),
            linear_color_stop(rgba(crate::kit::alpha(style.surface_rgb, band_end)), 1.0),
        ),
    ));
    let clear = style.wash_rgba & 0xffff_ff00;
    let half_streak = px(OVERFLOW_STREAK_WIDTH / 2.0);
    let streak_size = size(half_streak, px(OVERFLOW_STREAK_HEIGHT));
    let streak_top = centre_y - px(OVERFLOW_STREAK_HEIGHT / 2.0);
    let center_x = bounds.center().x;
    window.paint_quad(fill(
        Bounds::new(point(center_x - half_streak, streak_top), streak_size),
        linear_gradient(
            90.0,
            linear_color_stop(rgba(clear), 0.0),
            linear_color_stop(rgba(style.wash_rgba), 1.0),
        ),
    ));
    window.paint_quad(fill(
        Bounds::new(point(center_x, streak_top), streak_size),
        linear_gradient(
            90.0,
            linear_color_stop(rgba(style.wash_rgba), 0.0),
            linear_color_stop(rgba(clear), 1.0),
        ),
    ));
    let mut path = PathBuilder::stroke(px(OVERFLOW_CHEVRON_STROKE));
    path.move_to(point(center_x - half_chevron, wing_y));
    path.line_to(point(center_x, tip_y));
    path.line_to(point(center_x + half_chevron, wing_y));
    if let Ok(path) = path.build() {
        window.paint_path(path, rgba(style.ink_rgba));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        overflow_edges, OVERFLOW_CHEVRON_RISE, OVERFLOW_CHEVRON_STROKE, OVERFLOW_CUE_CENTRE,
        OVERFLOW_FADE_HEIGHT, OVERFLOW_STREAK_HEIGHT, OVERFLOW_STREAK_WIDTH,
    };
    use gpui::{point, px, size, Bounds, Pixels};

    fn viewport() -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(0.)), size(px(300.), px(400.)))
    }

    fn child(top: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(top)), size(px(300.), px(height)))
    }

    #[test]
    fn overflow_edges_stay_dark_at_rest() {
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(900., 52.), px(0.)),
            (false, true),
            "only the last child past the bottom lights"
        );
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(300., 52.), px(0.)),
            (false, false),
            "both edges stay dark when every child fits"
        );
    }

    #[test]
    fn overflow_edges_ignore_bottom_padding() {
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(332., 52.), px(0.)),
            (false, false),
            "padding below the last child does not light the bottom"
        );
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(348., 52.), px(0.)),
            (false, false),
            "a last child ending exactly at the viewport bottom stays dark"
        );
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(348., 52.), px(2.)),
            (false, true),
            "two pixels of the last child cut off light the bottom"
        );
    }

    #[test]
    fn overflow_edges_light_the_top_once_the_first_child_is_cut() {
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(300., 52.), px(-1.)),
            (false, false),
            "one pixel of overlap stays dark"
        );
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(300., 52.), px(-2.)),
            (true, false),
            "two pixels of the first child cut off light the top"
        );
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn the_cue_fits_inside_its_band() {
        assert!(OVERFLOW_STREAK_HEIGHT <= OVERFLOW_FADE_HEIGHT);
        assert_eq!(OVERFLOW_CUE_CENTRE, OVERFLOW_FADE_HEIGHT / 2.0);
        assert!(OVERFLOW_CUE_CENTRE + OVERFLOW_STREAK_HEIGHT / 2.0 <= OVERFLOW_FADE_HEIGHT);
        assert!(OVERFLOW_CHEVRON_RISE + OVERFLOW_CHEVRON_STROKE <= OVERFLOW_STREAK_HEIGHT);
        assert_eq!((OVERFLOW_STREAK_WIDTH / 2.0).fract(), 0.0);
    }
}
