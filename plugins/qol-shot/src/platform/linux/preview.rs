use crate::Rect;

pub fn grab_preview_rgba(rect: &Rect) -> Option<(Vec<u8>, u32, u32)> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

    if rect.w <= 0 || rect.h <= 0 {
        return None;
    }
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;
    let (w, h) = (rect.w as u16, rect.h as u16);
    let reply = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            root,
            rect.x as i16,
            rect.y as i16,
            w,
            h,
            u32::MAX,
        )
        .ok()?
        .reply()
        .ok()?;

    let mut data = reply.data;
    if data.len() != w as usize * h as usize * 4 {
        return None;
    }
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Some((data, w as u32, h as u32))
}

pub fn configure_preview_window(title: String) {
    super::window::configure_window_async(
        title,
        "SHOT_OVERLAY",
        qol_gpui::popup_window::configure_overlay_window,
    );
}
