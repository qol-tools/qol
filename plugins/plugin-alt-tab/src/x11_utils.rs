use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub preview_path: Option<String>,
}

pub fn estimate_window_count() -> usize {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return 0;
    };
    let root = conn.setup().roots[screen_num].root;

    let stacking_atom = conn
        .intern_atom(false, b"_NET_CLIENT_LIST_STACKING")
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| reply.atom)
        .unwrap_or(0);
    let fallback_atom = conn
        .intern_atom(false, b"_NET_CLIENT_LIST")
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| reply.atom)
        .unwrap_or(0);
    let list_atom = if stacking_atom != 0 {
        stacking_atom
    } else {
        fallback_atom
    };
    if list_atom == 0 {
        return 0;
    }

    conn.get_property(false, root, list_atom, AtomEnum::WINDOW, 0, 2048)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|prop| prop.value32().map(|values| values.count()))
        .unwrap_or(0)
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

        if !title.is_empty() {
            if title == "Desktop" {
                continue;
            }
            windows.push(WindowInfo {
                id,
                title,
                preview_path: None,
            });
        }
    }

    #[cfg(debug_assertions)]
    eprintln!("[x11] get_open_windows total results: {}", windows.len());

    windows
}
pub fn capture_preview(window_id: u32, max_w: usize, max_h: usize) -> Option<String> {
    let (data, width, height) = capture_preview_raw(window_id)?;
    process_and_save_preview(window_id, data, width, height, max_w, max_h)
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
    let rgba = to_rgba(&data, width, height)?;
    let (thumb, thumb_w, thumb_h) = downscale_rgba_keep_aspect(&rgba, width, height, max_w, max_h);

    let image = image::RgbaImage::from_raw(thumb_w as u32, thumb_h as u32, thumb)?;
    let cache_dir = preview_cache_dir()?;
    let path = cache_dir.join(format!("{}.png", window_id));
    image
        .save_with_format(&path, image::ImageFormat::Png)
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

fn to_rgba(data: &[u8], width: usize, height: usize) -> Option<Vec<u8>> {
    let pixels = width.checked_mul(height)?;
    if pixels == 0 {
        return None;
    }
    let bytes_per_pixel = data.len() / pixels;
    if bytes_per_pixel < 3 {
        return None;
    }

    let mut out = Vec::with_capacity(pixels * 4);
    for i in 0..pixels {
        let base = i * bytes_per_pixel;
        if base + 2 >= data.len() {
            return None;
        }
        // Common X11 little-endian format: B, G, R, [unused]
        let b = data[base];
        let g = data[base + 1];
        let r = data[base + 2];
        out.extend_from_slice(&[r, g, b, 255]);
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
