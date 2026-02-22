use super::WindowInfo;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};
use std::sync::Arc;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

#[derive(Clone, Copy)]
struct ChannelOrder {
    red: usize,
    green: usize,
    blue: usize,
}

impl Default for ChannelOrder {
    fn default() -> Self {
        Self {
            red: 2,
            green: 1,
            blue: 0,
        }
    }
}

struct LiveCaptureSession {
    conn: RustConnection,
    channel_order: ChannelOrder,
}

std::thread_local! {
    static LIVE_CAPTURE_SESSION: std::cell::RefCell<Option<LiveCaptureSession>> = std::cell::RefCell::new(None);
}

pub fn cached_preview_path(window_id: u32) -> Option<String> {
    let cache_dir = preview_cache_dir()?;
    let path = cache_dir.join(format!("{}.png", window_id));
    if !path.is_file() {
        return None;
    }
    Some(path.to_string_lossy().to_string())
}

pub fn activate_window(window_id: u32) {
    std::process::Command::new("xdotool")
        .arg("windowactivate")
        .arg(window_id.to_string())
        .status()
        .ok();
}

pub fn move_app_window(title: &str, x: i32, y: i32) {
    std::process::Command::new("xdotool")
        .arg("search")
        .arg("--name")
        .arg(title)
        .arg("windowmove")
        .arg(x.to_string())
        .arg(y.to_string())
        .status()
        .ok();
}

pub fn get_open_windows() -> Vec<WindowInfo> {
    let mut windows = Vec::new();

    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return windows;
    };

    let root = conn.setup().roots[screen_num].root;

    // Atoms we need
    let atoms = [
        "_NET_CLIENT_LIST",
        "_NET_CLIENT_LIST_STACKING",
        "_NET_WM_NAME",
        "UTF8_STRING",
        "_NET_WM_WINDOW_TYPE",
        "_NET_WM_WINDOW_TYPE_NORMAL",
        "_NET_WM_STATE",
        "_NET_WM_STATE_HIDDEN",
        "WM_CLASS",
    ];

    let mut cookies = Vec::new();
    for name in &atoms {
        cookies.push(conn.intern_atom(false, name.as_bytes()).ok());
    }

    let mut atom_map = std::collections::HashMap::new();
    for (i, cookie) in cookies.into_iter().enumerate() {
        if let Some(reply) = cookie.and_then(|c| c.reply().ok()) {
            atom_map.insert(atoms[i], reply.atom);
        }
    }

    let list_atom = atom_map
        .get("_NET_CLIENT_LIST_STACKING")
        .or_else(|| atom_map.get("_NET_CLIENT_LIST"))
        .copied()
        .unwrap_or(0);

    if list_atom == 0 {
        return windows;
    }

    let prop = conn
        .get_property(false, root, list_atom, AtomEnum::WINDOW, 0, 1024)
        .ok()
        .and_then(|c| c.reply().ok());

    let Some(prop) = prop else {
        return windows;
    };

    let Some(value32) = prop.value32() else {
        return windows;
    };

    let ids: Vec<u32> = value32.collect();
    if ids.is_empty() {
        return windows;
    }

    // Pipelined type checks
    let type_atom = atom_map.get("_NET_WM_WINDOW_TYPE").copied();
    let type_cookies: Vec<_> = ids
        .iter()
        .map(|&id| {
            if let Some(ta) = type_atom {
                conn.get_property(false, id, ta, AtomEnum::ATOM, 0, 10).ok()
            } else {
                None
            }
        })
        .collect();

    let normal_atom = atom_map
        .get("_NET_WM_WINDOW_TYPE_NORMAL")
        .copied()
        .unwrap_or(0);

    // Filter IDs by type
    let mut filtered_ids = Vec::new();
    for (i, cookie) in type_cookies.into_iter().enumerate() {
        let mut is_normal = true;
        if let Some(tp) = cookie.and_then(|c| c.reply().ok()) {
            if let Some(types) = tp.value32() {
                let mut has_any_type = false;
                let mut found_normal = false;
                for t in types {
                    has_any_type = true;
                    if t == normal_atom {
                        found_normal = true;
                        break;
                    }
                }
                if has_any_type && !found_normal {
                    is_normal = false;
                }
            }
        }
        if is_normal {
            filtered_ids.push(ids[i]);
        }
    }

    // Pipelined name requests for filtered IDs
    let net_name_atom = atom_map.get("_NET_WM_NAME").copied();
    let mut net_name_cookies = Vec::new();
    let mut wm_name_cookies = Vec::new();
    let mut wm_class_cookies = Vec::new();

    let wm_class_atom = atom_map.get("WM_CLASS").copied().unwrap_or(0);

    for &id in &filtered_ids {
        if let Some(na) = net_name_atom {
            net_name_cookies.push(
                conn.get_property(false, id, na, AtomEnum::ANY, 0, 1024)
                    .ok(),
            );
        } else {
            net_name_cookies.push(None);
        }
        wm_name_cookies.push(
            conn.get_property(false, id, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 1024)
                .ok(),
        );
        wm_class_cookies.push(
            conn.get_property(false, id, wm_class_atom, AtomEnum::STRING, 0, 1024)
                .ok(),
        );
    }

    for (i, &id) in filtered_ids.iter().enumerate().rev() {
        let mut title = String::new();

        // Check _NET_WM_NAME reply
        if let Some(reply) = net_name_cookies[i].take().and_then(|c| c.reply().ok()) {
            title = String::from_utf8_lossy(&reply.value).into_owned();
        }

        // Fallback to WM_NAME reply
        if title.is_empty() {
            if let Some(reply) = wm_name_cookies[i].take().and_then(|c| c.reply().ok()) {
                title = String::from_utf8_lossy(&reply.value).into_owned();
            }
        }

        let mut app_name = String::new();
        if let Some(reply) = wm_class_cookies[i].take().and_then(|c| c.reply().ok()) {
            // WM_CLASS is a null-separated string: "instance\0class\0"
            // We want the class part (the second one) if it exists, otherwise the first.
            let parts: Vec<&str> = std::str::from_utf8(&reply.value)
                .unwrap_or("")
                .split('\0')
                .filter(|s| !s.is_empty())
                .collect();
            if parts.len() >= 2 {
                app_name = parts[1].to_string();
            } else if !parts.is_empty() {
                app_name = parts[0].to_string();
            }
        }

        if !title.is_empty() {
            if title == "Desktop" {
                continue;
            }
            windows.push(WindowInfo {
                id,
                title,
                app_name,
                preview_path: None,
                preview_frame: None,
            });
        }
    }

    #[cfg(debug_assertions)]
    eprintln!("[x11] get_open_windows total results: {}", windows.len());

    windows
}
pub fn capture_preview(window_id: u32, max_w: usize, max_h: usize) -> Option<String> {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return None;
    };
    let geometry = conn.get_geometry(window_id).ok()?.reply().ok()?;
    if geometry.width == 0 || geometry.height == 0 {
        return None;
    }
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            window_id,
            0,
            0,
            geometry.width,
            geometry.height,
            u32::MAX,
        )
        .ok()?
        .reply()
        .ok()?;
    let channel_order = detect_channel_order(&conn, screen_num);
    process_and_save_preview_with_order(
        window_id,
        image.data.to_vec(),
        geometry.width as usize,
        geometry.height as usize,
        max_w,
        max_h,
        channel_order,
    )
}

pub fn capture_previews_batch(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<String>)> {
    if targets.is_empty() {
        return Vec::new();
    }

    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return targets.iter().map(|(i, _)| (*i, None)).collect();
    };
    let channel_order = detect_channel_order(&conn, screen_num);

    let window_ids: Vec<u32> = targets.iter().map(|(_, id)| *id).collect();

    let geometry_cookies: Vec<_> = window_ids
        .iter()
        .map(|&id| conn.get_geometry(id).ok())
        .collect();
    let geometries: Vec<_> = geometry_cookies
        .into_iter()
        .map(|cookie| cookie.and_then(|cookie| cookie.reply().ok()))
        .collect();

    let mut image_cookies: Vec<_> = window_ids
        .iter()
        .enumerate()
        .map(|(pos, &id)| {
            let geo = geometries[pos].as_ref()?;
            if geo.width == 0 || geo.height == 0 {
                return None;
            }
            conn.get_image(
                ImageFormat::Z_PIXMAP,
                id,
                0,
                0,
                geo.width,
                geo.height,
                u32::MAX,
            )
            .ok()
        })
        .collect();

    let mut out = Vec::with_capacity(targets.len());
    for (pos, (list_index, window_id)) in targets.iter().copied().enumerate() {
        let Some(geo) = geometries[pos].as_ref() else {
            out.push((list_index, None));
            continue;
        };
        let Some(reply) = image_cookies[pos]
            .take()
            .and_then(|cookie| cookie.reply().ok())
        else {
            out.push((list_index, None));
            continue;
        };

        let path = process_and_save_preview_with_order(
            window_id,
            reply.data.to_vec(),
            geo.width as usize,
            geo.height as usize,
            max_w,
            max_h,
            channel_order,
        );
        out.push((list_index, path));
    }

    out
}

pub fn capture_preview_frame(window_id: u32, max_w: usize, max_h: usize) -> Option<super::PreviewFrame> {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return None;
    };
    let geometry = conn.get_geometry(window_id).ok()?.reply().ok()?;
    if geometry.width == 0 || geometry.height == 0 {
        return None;
    }
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            window_id,
            0,
            0,
            geometry.width,
            geometry.height,
            u32::MAX,
        )
        .ok()?
        .reply()
        .ok()?;
    let channel_order = detect_channel_order(&conn, screen_num);
    let rgba = to_bgra_with_order(
        &image.data,
        geometry.width as usize,
        geometry.height as usize,
        channel_order,
    )?;
    let (thumb, thumb_w, thumb_h) = downscale_rgba_keep_aspect(
        &rgba,
        geometry.width as usize,
        geometry.height as usize,
        max_w,
        max_h,
    );
    Some(super::PreviewFrame {
        rgba: Arc::new(thumb),
        width: thumb_w as u32,
        height: thumb_h as u32,
    })
}

pub fn capture_frames_batch(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<super::PreviewFrame>)> {
    if targets.is_empty() {
        return Vec::new();
    }

    LIVE_CAPTURE_SESSION.with(|cell| {
        let mut session_slot = cell.borrow_mut();
        if session_slot.is_none() {
            *session_slot = connect_live_capture_session();
        }

        let mut frames = if let Some(session) = session_slot.as_ref() {
            capture_frames_with_connection(
                &session.conn,
                targets,
                max_w,
                max_h,
                session.channel_order,
            )
        } else {
            targets.iter().map(|(i, _)| (*i, None)).collect()
        };

        let should_retry = frames.iter().all(|(_, frame)| frame.is_none());
        if should_retry {
            *session_slot = connect_live_capture_session();
            if let Some(session) = session_slot.as_ref() {
                frames = capture_frames_with_connection(
                    &session.conn,
                    targets,
                    max_w,
                    max_h,
                    session.channel_order,
                );
            }
        }

        frames
    })
}

fn connect_live_capture_session() -> Option<LiveCaptureSession> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let channel_order = detect_channel_order(&conn, screen_num);
    Some(LiveCaptureSession { conn, channel_order })
}

fn detect_channel_order<C: Connection>(conn: &C, screen_num: usize) -> ChannelOrder {
    let Some(screen) = conn.setup().roots.get(screen_num) else {
        return ChannelOrder::default();
    };
    let Some(visual) = screen
        .allowed_depths
        .iter()
        .flat_map(|depth| depth.visuals.iter())
        .find(|visual| visual.visual_id == screen.root_visual)
    else {
        return ChannelOrder::default();
    };

    let red = (visual.red_mask.trailing_zeros() / 8) as usize;
    let green = (visual.green_mask.trailing_zeros() / 8) as usize;
    let blue = (visual.blue_mask.trailing_zeros() / 8) as usize;
    if red > 3 || green > 3 || blue > 3 {
        return ChannelOrder::default();
    }

    ChannelOrder { red, green, blue }
}

fn capture_frames_with_connection<C: Connection>(
    conn: &C,
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
    channel_order: ChannelOrder,
) -> Vec<(usize, Option<super::PreviewFrame>)> {
    let window_ids: Vec<u32> = targets.iter().map(|(_, id)| *id).collect();

    let geometry_cookies: Vec<_> = window_ids
        .iter()
        .map(|&id| conn.get_geometry(id).ok())
        .collect();
    let geometries: Vec<_> = geometry_cookies
        .into_iter()
        .map(|cookie| cookie.and_then(|cookie| cookie.reply().ok()))
        .collect();

    let mut image_cookies: Vec<_> = window_ids
        .iter()
        .enumerate()
        .map(|(pos, &id)| {
            let geo = geometries[pos].as_ref()?;
            if geo.width == 0 || geo.height == 0 {
                return None;
            }
            conn.get_image(
                ImageFormat::Z_PIXMAP,
                id,
                0,
                0,
                geo.width,
                geo.height,
                u32::MAX,
            )
            .ok()
        })
        .collect();

    let mut out = Vec::with_capacity(targets.len());
    for (pos, (list_index, _)) in targets.iter().copied().enumerate() {
        let Some(geo) = geometries[pos].as_ref() else {
            out.push((list_index, None));
            continue;
        };
        let Some(reply) = image_cookies[pos]
            .take()
            .and_then(|cookie| cookie.reply().ok())
        else {
            out.push((list_index, None));
            continue;
        };

        let width = geo.width as usize;
        let height = geo.height as usize;
        let frame = to_bgra_with_order(&reply.data, width, height, channel_order).and_then(|rgba| {
            let (thumb, tw, th) = downscale_rgba_keep_aspect(&rgba, width, height, max_w, max_h);
            Some(super::PreviewFrame {
                rgba: Arc::new(thumb),
                width: tw as u32,
                height: th as u32,
            })
        });
        out.push((list_index, frame));
    }

    out
}

pub fn capture_preview_raw(window_id: u32) -> Option<(Vec<u8>, usize, usize)> {
    let Ok((conn, _)) = x11rb::connect(None) else {
        return None;
    };

    let geometry = conn.get_geometry(window_id).ok()?.reply().ok()?;
    if geometry.width == 0 || geometry.height == 0 {
        return None;
    }

    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            window_id,
            0,
            0,
            geometry.width,
            geometry.height,
            u32::MAX,
        )
        .ok()?
        .reply()
        .ok()?;

    Some((
        image.data.to_vec(),
        geometry.width as usize,
        geometry.height as usize,
    ))
}

/// Process raw image data and save as PNG thumbnail.

pub fn process_and_save_preview(
    window_id: u32,
    data: Vec<u8>,
    width: usize,
    height: usize,
    max_w: usize,
    max_h: usize,
) -> Option<String> {
    process_and_save_preview_with_order(
        window_id,
        data,
        width,
        height,
        max_w,
        max_h,
        ChannelOrder::default(),
    )
}

fn process_and_save_preview_with_order(
    window_id: u32,
    data: Vec<u8>,
    width: usize,
    height: usize,
    max_w: usize,
    max_h: usize,
    channel_order: ChannelOrder,
) -> Option<String> {
    let rgba = to_rgba_with_order(&data, width, height, channel_order)?;
    let (thumb, thumb_w, thumb_h) = downscale_rgba_keep_aspect(&rgba, width, height, max_w, max_h);

    let cache_dir = preview_cache_dir()?;
    let path = cache_dir.join(format!("{}.png", window_id));
    let file = std::fs::File::create(&path).ok()?;
    let writer = std::io::BufWriter::new(file);
    let encoder = PngEncoder::new_with_quality(writer, CompressionType::Fast, FilterType::NoFilter);
    encoder
        .write_image(
            &thumb,
            thumb_w as u32,
            thumb_h as u32,
            ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(path.to_string_lossy().to_string())
}

fn preview_cache_dir() -> Option<std::path::PathBuf> {
    let base = dirs::cache_dir().or_else(|| Some(std::env::temp_dir()))?;
    let path = base
        .join("qol-tray")
        .join("plugin-alt-tab")
        .join("previews");
    std::fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn to_rgba_with_order(
    data: &[u8],
    width: usize,
    height: usize,
    channel_order: ChannelOrder,
) -> Option<Vec<u8>> {
    let pixels = width.checked_mul(height)?;
    if pixels == 0 {
        return None;
    }
    let bytes_per_pixel = data.len() / pixels;
    if bytes_per_pixel < 3 {
        return None;
    }
    if channel_order.red >= bytes_per_pixel
        || channel_order.green >= bytes_per_pixel
        || channel_order.blue >= bytes_per_pixel
    {
        return None;
    }

    let mut out = Vec::with_capacity(pixels * 4);
    for i in 0..pixels {
        let base = i * bytes_per_pixel;
        let r_idx = base + channel_order.red;
        let g_idx = base + channel_order.green;
        let b_idx = base + channel_order.blue;
        if r_idx >= data.len() || g_idx >= data.len() || b_idx >= data.len() {
            return None;
        }
        let r = data[r_idx];
        let g = data[g_idx];
        let b = data[b_idx];
        out.extend_from_slice(&[r, g, b, 255]);
    }
    Some(out)
}

fn to_bgra_with_order(
    data: &[u8],
    width: usize,
    height: usize,
    channel_order: ChannelOrder,
) -> Option<Vec<u8>> {
    let pixels = width.checked_mul(height)?;
    if pixels == 0 {
        return None;
    }
    let bytes_per_pixel = data.len() / pixels;
    if bytes_per_pixel < 3 {
        return None;
    }
    if channel_order.red >= bytes_per_pixel
        || channel_order.green >= bytes_per_pixel
        || channel_order.blue >= bytes_per_pixel
    {
        return None;
    }

    let mut out = Vec::with_capacity(pixels * 4);
    for i in 0..pixels {
        let base = i * bytes_per_pixel;
        let r_idx = base + channel_order.red;
        let g_idx = base + channel_order.green;
        let b_idx = base + channel_order.blue;
        if r_idx >= data.len() || g_idx >= data.len() || b_idx >= data.len() {
            return None;
        }
        let r = data[r_idx];
        let g = data[g_idx];
        let b = data[b_idx];
        out.extend_from_slice(&[b, g, r, 255]);
    }
    Some(out)
}

fn downscale_rgba_keep_aspect(
    rgba: &[u8],
    src_w: usize,
    src_h: usize,
    max_w: usize,
    max_h: usize,
) -> (Vec<u8>, usize, usize) {
    if src_w == 0 || src_h == 0 || max_w == 0 || max_h == 0 {
        return (Vec::new(), 0, 0);
    }

    let scale_w = max_w as f32 / src_w as f32;
    let scale_h = max_h as f32 / src_h as f32;
    let scale = scale_w.min(scale_h).min(1.0);

    let scaled_w = ((src_w as f32 * scale).round() as usize).max(1).min(max_w);
    let scaled_h = ((src_h as f32 * scale).round() as usize).max(1).min(max_h);

    // Always return a fixed-size canvas so every preview tile has identical dimensions.
    let mut canvas = vec![0u8; max_w * max_h * 4];
    let offset_x = (max_w - scaled_w) / 2;
    let offset_y = (max_h - scaled_h) / 2;

    for y in 0..scaled_h {
        let src_y = (y * src_h) / scaled_h;
        for x in 0..scaled_w {
            let src_x = (x * src_w) / scaled_w;
            let src_i = (src_y * src_w + src_x) * 4;
            let dst_x = offset_x + x;
            let dst_y = offset_y + y;
            let dst_i = (dst_y * max_w + dst_x) * 4;
            canvas[dst_i..dst_i + 4].copy_from_slice(&rgba[src_i..src_i + 4]);
        }
    }

    (canvas, max_w, max_h)
}
