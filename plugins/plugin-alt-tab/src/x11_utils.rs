use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub preview_path: Option<String>,
}

pub fn get_open_windows() -> Vec<WindowInfo> {
    let mut windows = Vec::new();

    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return windows;
    };

    let root = conn.setup().roots[screen_num].root;

    let client_list_atom = conn
        .intern_atom(false, b"_NET_CLIENT_LIST")
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .unwrap_or(0);

    if client_list_atom == 0 {
        return windows;
    }

    let net_wm_name_atom = conn
        .intern_atom(false, b"_NET_WM_NAME")
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .unwrap_or(0);

    let prop = conn
        .get_property(false, root, client_list_atom, AtomEnum::WINDOW, 0, 1024)
        .ok()
        .and_then(|c| c.reply().ok());

    if let Some(prop) = prop {
        if let Some(value32) = prop.value32() {
            for window_id in value32 {
                let mut title = String::new();

                // Try _NET_WM_NAME first
                if net_wm_name_atom != 0 {
                    let name_prop = conn
                        .get_property(false, window_id, net_wm_name_atom, AtomEnum::ANY, 0, 1024)
                        .ok()
                        .and_then(|c| c.reply().ok());

                    if let Some(np) = name_prop {
                        title = String::from_utf8_lossy(&np.value).into_owned();
                    }
                }

                // Fallback to WM_NAME
                if title.is_empty() {
                    let name_prop = conn
                        .get_property(false, window_id, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 1024)
                        .ok()
                        .and_then(|c| c.reply().ok());
                    if let Some(np) = name_prop {
                        title = String::from_utf8_lossy(&np.value).into_owned();
                    }
                }

                if !title.is_empty() {
                    let preview_path = capture_window_preview_path(&conn, window_id);
                    windows.push(WindowInfo {
                        id: window_id,
                        title,
                        preview_path,
                    });
                }
            }
        }
    }

    windows
}

fn capture_window_preview_path(conn: &RustConnection, window_id: Window) -> Option<String> {
    let geometry = conn
        .get_geometry(window_id)
        .ok()
        .and_then(|cookie| cookie.reply().ok())?;

    let width = geometry.width as usize;
    let height = geometry.height as usize;
    if width == 0 || height == 0 {
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
        .ok()
        .and_then(|cookie| cookie.reply().ok())?;

    let rgba = to_rgba(&image.data, width, height)?;
    let (thumb, thumb_w, thumb_h) = downscale_rgba_keep_aspect(&rgba, width, height, 240, 140);

    let image = image::RgbaImage::from_raw(thumb_w as u32, thumb_h as u32, thumb)?;
    let cache_dir = preview_cache_dir()?;
    let path = cache_dir.join(format!("{}.png", window_id));
    image.save_with_format(&path, image::ImageFormat::Png).ok()?;
    Some(path.to_string_lossy().to_string())
}

fn preview_cache_dir() -> Option<std::path::PathBuf> {
    let base = dirs::cache_dir().or_else(|| Some(std::env::temp_dir()))?;
    let path = base.join("qol-tray").join("plugin-alt-tab").join("previews");
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
    if src_w == 0 || src_h == 0 {
        return (Vec::new(), 0, 0);
    }

    let scale_w = max_w as f32 / src_w as f32;
    let scale_h = max_h as f32 / src_h as f32;
    let scale = scale_w.min(scale_h).min(1.0);

    let dst_w = ((src_w as f32 * scale).round() as usize).max(1);
    let dst_h = ((src_h as f32 * scale).round() as usize).max(1);

    if dst_w == src_w && dst_h == src_h {
        return (rgba.to_vec(), src_w, src_h);
    }

    let mut out = vec![0u8; dst_w * dst_h * 4];
    for y in 0..dst_h {
        let src_y = (y * src_h) / dst_h;
        for x in 0..dst_w {
            let src_x = (x * src_w) / dst_w;
            let src_i = (src_y * src_w + src_x) * 4;
            let dst_i = (y * dst_w + x) * 4;
            out[dst_i..dst_i + 4].copy_from_slice(&rgba[src_i..src_i + 4]);
        }
    }

    (out, dst_w, dst_h)
}
