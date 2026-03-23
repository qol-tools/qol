use crate::discovery::WindowInfo;
use qol_plugin_api::app_icon::RgbaImage;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as _, Redirect};
use x11rb::protocol::render::{self, ConnectionExt as RenderExt, Pictformat};
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};
use x11rb::rust_connection::RustConnection;

struct CaptureSession {
    conn: RustConnection,
    root: u32,
    formats: HashMap<u8, Pictformat>,
}

fn capture_session() -> &'static Mutex<Option<CaptureSession>> {
    static SESSION: OnceLock<Mutex<Option<CaptureSession>>> = OnceLock::new();
    SESSION.get_or_init(|| Mutex::new(connect_session()))
}

fn connect_session() -> Option<CaptureSession> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    conn.composite_query_version(0, 4).ok()?.reply().ok()?;
    conn.render_query_version(0, 11).ok()?.reply().ok()?;
    let root = conn.setup().roots[screen_num].root;
    let pict_reply = conn.render_query_pict_formats().ok()?.reply().ok()?;
    let formats = build_format_map(&pict_reply, screen_num);
    Some(CaptureSession {
        conn,
        root,
        formats,
    })
}

fn build_format_map(
    reply: &render::QueryPictFormatsReply,
    screen_num: usize,
) -> HashMap<u8, Pictformat> {
    let mut map = HashMap::new();
    if let Some(screen) = reply.screens.get(screen_num) {
        for depth_info in &screen.depths {
            if let Some(vis) = depth_info.visuals.first() {
                map.insert(depth_info.depth, vis.format);
            }
        }
    }
    for f in &reply.formats {
        if f.type_ == render::PictType::DIRECT {
            map.entry(f.depth).or_insert(f.id);
        }
    }
    map
}

pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    let Ok(mut guard) = capture_session().lock() else {
        return targets.iter().map(|&(idx, _)| (idx, None)).collect();
    };
    let session = match &*guard {
        Some(_) => guard.as_ref().unwrap(),
        None => {
            *guard = connect_session();
            match &*guard {
                Some(s) => s,
                None => return targets.iter().map(|&(idx, _)| (idx, None)).collect(),
            }
        }
    };
    let results: Vec<_> = targets
        .iter()
        .map(|&(idx, wid)| (idx, capture_window_scaled(session, wid, max_w, max_h)))
        .collect();
    if results.iter().all(|(_, r)| r.is_none()) && !targets.is_empty() {
        *guard = connect_session();
    }
    results
}

fn capture_window_scaled(
    session: &CaptureSession,
    wid: u32,
    max_w: usize,
    max_h: usize,
) -> Option<RgbaImage> {
    let conn = &session.conn;
    let geom = conn.get_geometry(wid).ok()?.reply().ok()?;
    let (src_w, src_h) = (geom.width as usize, geom.height as usize);
    if src_w == 0 || src_h == 0 {
        return None;
    }

    let fmt = session.formats.get(&geom.depth).copied()?;

    let scale = (max_w as f64 / src_w as f64)
        .min(max_h as f64 / src_h as f64)
        .min(1.0);
    let dst_w = ((src_w as f64 * scale).round() as u16).max(1);
    let dst_h = ((src_h as f64 * scale).round() as u16).max(1);

    let raw = render_scaled_capture(
        session,
        wid,
        fmt,
        geom.depth,
        geom.width,
        geom.height,
        dst_w,
        dst_h,
    )?;

    let force_opaque = geom.depth < 32;
    let offset_x = (max_w - dst_w as usize) / 2;
    let offset_y = (max_h - dst_h as usize) / 2;
    let mut out = vec![0u8; max_w * max_h * 4];
    for y in 0..dst_h as usize {
        for x in 0..dst_w as usize {
            let s = (y * dst_w as usize + x) * 4;
            let d = ((offset_y + y) * max_w + offset_x + x) * 4;
            if s + 4 <= raw.len() {
                out[d..d + 3].copy_from_slice(&raw[s..s + 3]);
                out[d + 3] = if force_opaque { 255 } else { raw[s + 3] };
            }
        }
    }
    Some(RgbaImage {
        data: out,
        width: max_w,
        height: max_h,
    })
}

fn render_scaled_capture(
    session: &CaptureSession,
    wid: u32,
    fmt: Pictformat,
    depth: u8,
    src_w: u16,
    src_h: u16,
    dst_w: u16,
    dst_h: u16,
) -> Option<Vec<u8>> {
    let conn = &session.conn;

    let redirect = conn
        .composite_redirect_window(wid, Redirect::AUTOMATIC)
        .ok()?;
    let src_pixmap = conn.generate_id().ok()?;
    let name = conn.composite_name_window_pixmap(wid, src_pixmap).ok()?;
    redirect.check().ok()?;
    if name.check().is_err() {
        let _ = conn.composite_unredirect_window(wid, Redirect::AUTOMATIC);
        return None;
    }

    let result = scale_via_render(session, src_pixmap, fmt, depth, src_w, src_h, dst_w, dst_h);

    let _ = conn.free_pixmap(src_pixmap);
    let _ = conn.composite_unredirect_window(wid, Redirect::AUTOMATIC);
    result
}

fn scale_via_render(
    session: &CaptureSession,
    src_pixmap: u32,
    fmt: Pictformat,
    depth: u8,
    src_w: u16,
    src_h: u16,
    dst_w: u16,
    dst_h: u16,
) -> Option<Vec<u8>> {
    let conn = &session.conn;

    let dst_pixmap = conn.generate_id().ok()?;
    if conn
        .create_pixmap(depth, dst_pixmap, session.root, dst_w, dst_h)
        .ok()?
        .check()
        .is_err()
    {
        return None;
    }

    let src_pic = conn.generate_id().ok()?;
    let dst_pic = conn.generate_id().ok()?;
    let aux = render::CreatePictureAux::new();

    let src_ok = conn
        .render_create_picture(src_pic, src_pixmap, fmt, &aux)
        .ok()
        .and_then(|c| c.check().ok());
    let dst_ok = conn
        .render_create_picture(dst_pic, dst_pixmap, fmt, &aux)
        .ok()
        .and_then(|c| c.check().ok());

    if src_ok.is_none() || dst_ok.is_none() {
        let _ = conn.render_free_picture(src_pic);
        let _ = conn.render_free_picture(dst_pic);
        let _ = conn.free_pixmap(dst_pixmap);
        return None;
    }

    let sx = ((src_w as i64) << 16) / dst_w as i64;
    let sy = ((src_h as i64) << 16) / dst_h as i64;
    let transform = render::Transform {
        matrix11: sx as i32,
        matrix12: 0,
        matrix13: 0,
        matrix21: 0,
        matrix22: sy as i32,
        matrix23: 0,
        matrix31: 0,
        matrix32: 0,
        matrix33: 1 << 16,
    };
    let _ = conn.render_set_picture_transform(src_pic, transform);
    let _ = conn.render_set_picture_filter(src_pic, b"bilinear", &[]);

    let _ = conn.render_composite(
        render::PictOp::SRC,
        src_pic,
        0u32,
        dst_pic,
        0,
        0,
        0,
        0,
        0,
        0,
        dst_w,
        dst_h,
    );

    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            dst_pixmap,
            0,
            0,
            dst_w,
            dst_h,
            u32::MAX,
        )
        .ok()
        .and_then(|c| c.reply().ok());

    let _ = conn.render_free_picture(src_pic);
    let _ = conn.render_free_picture(dst_pic);
    let _ = conn.free_pixmap(dst_pixmap);

    image.map(|img| img.data)
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
