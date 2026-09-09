use gpui::*;

pub const SEAM_TRACK_WIDTH: f32 = 3.0;
pub const SEAM_TRACK_INSET: f32 = 2.0;
pub const SEAM_THUMB_MIN: f32 = 24.0;

pub const OVERFLOW_FADE_HEIGHT: f32 = qol_theme::HEIGHT_HINT_BAR;
pub const OVERFLOW_DISC_SIZE: f32 = qol_theme::HEIGHT_INLINE;
pub const OVERFLOW_DISC_INSET: f32 = qol_theme::SPACE_INSET;
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
    pub hairline_rgba: u32,
}

pub fn overflow_fade(handle: ScrollHandle, style: OverflowFadeStyle) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let max = handle.max_offset().height;
            if max <= px(0.) {
                return;
            }
            let offset = handle.offset().y;
            if -offset > px(1.) {
                paint_overflow_edge(window, bounds, OverflowEdge::Top, style);
            }
            if max + offset > px(1.) {
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
    let disc = px(OVERFLOW_DISC_SIZE);
    let half_disc = px(OVERFLOW_DISC_SIZE / 2.0);
    let inset = px(OVERFLOW_DISC_INSET);
    let rise = px(OVERFLOW_CHEVRON_RISE);
    let half_chevron = px(OVERFLOW_CHEVRON_WIDTH / 2.0);
    let (band_top, disc_top, tip_y, wing_y, band_start, band_end, wash_start, wash_end) = match edge
    {
        OverflowEdge::Top => {
            let disc_top = bounds.top() + inset;
            let centre_y = disc_top + half_disc;
            (
                bounds.top(),
                disc_top,
                centre_y - rise * 0.5,
                centre_y + rise * 0.5,
                0xff,
                0x00,
                style.wash_rgba,
                style.wash_rgba & 0xffff_ff00,
            )
        }
        OverflowEdge::Bottom => {
            let disc_top = bounds.bottom() - inset - disc;
            let centre_y = disc_top + half_disc;
            (
                bounds.bottom() - fade_h,
                disc_top,
                centre_y + rise * 0.5,
                centre_y - rise * 0.5,
                0x00,
                0xff,
                style.wash_rgba & 0xffff_ff00,
                style.wash_rgba,
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
    window.paint_quad(quad(
        Bounds::new(
            point(bounds.center().x - half_disc, disc_top),
            size(disc, disc),
        ),
        Corners::all(half_disc),
        linear_gradient(
            180.0,
            linear_color_stop(rgba(wash_start), 0.0),
            linear_color_stop(rgba(wash_end), 1.0),
        ),
        Edges::all(px(1.0)),
        rgba(style.hairline_rgba),
        BorderStyle::default(),
    ));
    let mut path = PathBuilder::stroke(px(OVERFLOW_CHEVRON_STROKE));
    let center_x = bounds.center().x;
    path.move_to(point(center_x - half_chevron, wing_y));
    path.line_to(point(center_x, tip_y));
    path.line_to(point(center_x + half_chevron, wing_y));
    if let Ok(path) = path.build() {
        window.paint_path(path, rgba(style.ink_rgba));
    }
}
