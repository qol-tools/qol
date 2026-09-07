use gpui::*;

pub const SEAM_TRACK_WIDTH: f32 = 3.0;
pub const SEAM_TRACK_INSET: f32 = 2.0;
pub const SEAM_THUMB_MIN: f32 = 24.0;

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
