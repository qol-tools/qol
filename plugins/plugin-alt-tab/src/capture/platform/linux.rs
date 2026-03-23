use crate::discovery::WindowInfo;
use qol_plugin_api::app_icon::RgbaImage;
use std::sync::{Mutex, OnceLock};
use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as _, Redirect};
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};
use x11rb::rust_connection::RustConnection;

fn capture_conn() -> &'static Mutex<Option<RustConnection>> {
    static CONN: OnceLock<Mutex<Option<RustConnection>>> = OnceLock::new();
    CONN.get_or_init(|| Mutex::new(connect_with_composite()))
}

fn connect_with_composite() -> Option<RustConnection> {
    let (conn, _) = x11rb::connect(None).ok()?;
    conn.composite_query_version(0, 4).ok()?.reply().ok()?;
    Some(conn)
}

pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    let Ok(mut guard) = capture_conn().lock() else {
        return targets.iter().map(|&(idx, _)| (idx, None)).collect();
    };
    let conn = match &*guard {
        Some(c) => c,
        None => {
            *guard = connect_with_composite();
            match &*guard {
                Some(c) => c,
                None => return targets.iter().map(|&(idx, _)| (idx, None)).collect(),
            }
        }
    };
    let results: Vec<_> = targets
        .iter()
        .map(|&(idx, wid)| (idx, capture_window(conn, wid, max_w, max_h)))
        .collect();
    if results.iter().all(|(_, r)| r.is_none()) && !targets.is_empty() {
        *guard = connect_with_composite();
    }
    results
}

fn capture_window(
    conn: &RustConnection,
    wid: u32,
    max_w: usize,
    max_h: usize,
) -> Option<RgbaImage> {
    let geom = conn.get_geometry(wid).ok()?.reply().ok()?;
    let (src_w, src_h) = (geom.width as usize, geom.height as usize);
    if src_w == 0 || src_h == 0 {
        return None;
    }
    let (raw, depth) = read_pixmap_pixels(conn, wid, geom.width, geom.height)?;
    Some(scale_bgra(&raw, src_w, src_h, max_w, max_h, depth < 32))
}

fn read_pixmap_pixels(
    conn: &RustConnection,
    wid: u32,
    width: u16,
    height: u16,
) -> Option<(Vec<u8>, u8)> {
    let redirect = conn
        .composite_redirect_window(wid, Redirect::AUTOMATIC)
        .ok()?;
    let pixmap = conn.generate_id().ok()?;
    let name = conn.composite_name_window_pixmap(wid, pixmap).ok()?;
    redirect.check().ok()?;
    if name.check().is_err() {
        let _ = conn.composite_unredirect_window(wid, Redirect::AUTOMATIC);
        return None;
    }
    let image = conn
        .get_image(ImageFormat::Z_PIXMAP, pixmap, 0, 0, width, height, u32::MAX)
        .ok()
        .and_then(|c| c.reply().ok());
    let _ = conn.free_pixmap(pixmap);
    let _ = conn.composite_unredirect_window(wid, Redirect::AUTOMATIC);
    let img = image?;
    Some((img.data, img.depth))
}

fn scale_bgra(
    raw: &[u8],
    src_w: usize,
    src_h: usize,
    max_w: usize,
    max_h: usize,
    force_opaque: bool,
) -> RgbaImage {
    let scale = (max_w as f32 / src_w as f32)
        .min(max_h as f32 / src_h as f32)
        .min(1.0);
    let dst_w = ((src_w as f32 * scale).round() as usize).max(1).min(max_w);
    let dst_h = ((src_h as f32 * scale).round() as usize).max(1).min(max_h);
    let offset_x = (max_w - dst_w) / 2;
    let offset_y = (max_h - dst_h) / 2;
    let src_stride = src_w * 4;
    let mut out = vec![0u8; max_w * max_h * 4];
    for y in 0..dst_h {
        let src_y = (y * src_h) / dst_h;
        for x in 0..dst_w {
            let src_x = (x * src_w) / dst_w;
            let s = src_y * src_stride + src_x * 4;
            let d = ((offset_y + y) * max_w + offset_x + x) * 4;
            if s + 4 <= raw.len() {
                out[d..d + 3].copy_from_slice(&raw[s..s + 3]);
                out[d + 3] = if force_opaque { 255 } else { raw[s + 3] };
            }
        }
    }
    RgbaImage {
        data: out,
        width: max_w,
        height: max_h,
    }
}

pub fn get_app_icons(windows: &[WindowInfo]) -> std::collections::HashMap<String, RgbaImage> {
    let mut icons = std::collections::HashMap::new();
    for win in windows {
        if icons.contains_key(&win.app_name) {
            continue;
        }
        if let Some(ref icon) = win.icon {
            icons.insert(win.app_name.clone(), icon.clone());
        }
    }
    icons
}
