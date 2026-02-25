use super::RgbaImage;
use super::WindowInfo;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::xproto::*;

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

pub fn picker_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::PopUp
}

pub fn dismiss_picker(window: &mut gpui::Window) {
    window.minimize_window();
}

pub fn is_modifier_held() -> bool {
    let Ok((conn, _)) = x11rb::connect(None) else {
        return false;
    };
    let Ok(reply) = conn.query_keymap() else {
        return false;
    };
    let Ok(keymap) = reply.reply() else {
        return false;
    };
    let alt_l_held = keymap.keys[64 / 8] & (1 << (64 % 8)) != 0;
    let alt_r_held = keymap.keys[108 / 8] & (1 << (108 % 8)) != 0;
    alt_l_held || alt_r_held
}

pub fn is_shift_held() -> bool {
    let Ok((conn, _)) = x11rb::connect(None) else {
        return false;
    };
    let Ok(reply) = conn.query_keymap() else {
        return false;
    };
    let Ok(keymap) = reply.reply() else {
        return false;
    };
    // Shift_L = keycode 50, Shift_R = keycode 62
    let shift_l = keymap.keys[50 / 8] & (1 << (50 % 8)) != 0;
    let shift_r = keymap.keys[62 / 8] & (1 << (62 % 8)) != 0;
    shift_l || shift_r
}

pub fn activate_window(window_id: u32) {
    std::process::Command::new("xdotool")
        .arg("windowactivate")
        .arg(window_id.to_string())
        .status()
        .ok();
}

pub fn move_app_window(title: &str, x: i32, y: i32) -> bool {
    std::process::Command::new("xdotool")
        .arg("search")
        .arg("--name")
        .arg(title)
        .arg("windowmove")
        .arg(x.to_string())
        .arg(y.to_string())
        .status()
        .ok()
        .is_some_and(|s| s.success())
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
        "_NET_WM_ICON",
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
    let mut icon_cookies = Vec::new();

    let wm_class_atom = atom_map.get("WM_CLASS").copied().unwrap_or(0);
    let wm_icon_atom = atom_map.get("_NET_WM_ICON").copied().unwrap_or(0);

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
        if wm_icon_atom != 0 {
            icon_cookies.push(
                conn.get_property(false, id, wm_icon_atom, AtomEnum::CARDINAL, 0, 65536)
                    .ok(),
            );
        } else {
            icon_cookies.push(None);
        }
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

        let icon = icon_cookies[i]
            .take()
            .and_then(|c| c.reply().ok())
            .and_then(|reply| extract_x11_icon(&reply));

        if !title.is_empty() {
            if title == "Desktop" {
                continue;
            }
            windows.push(WindowInfo {
                id,
                title,
                app_name,
                preview_path: None,
                icon,
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            });
        }
    }

    #[cfg(debug_assertions)]
    eprintln!("[x11] get_open_windows total results: {}", windows.len());

    windows
}
pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    _max_w: usize,
    _max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    targets.iter().map(|&(idx, _)| (idx, None)).collect()
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

fn extract_x11_icon(reply: &GetPropertyReply) -> Option<RgbaImage> {
    let values: Vec<u32> = reply.value32()?.collect();
    if values.len() < 2 {
        return None;
    }

    // _NET_WM_ICON: width, height, ARGB pixels...  (may contain multiple sizes)
    // Pick the first icon ≤ 48px, or the smallest available.
    let mut offset = 0;
    let mut best: Option<(usize, usize, usize)> = None; // (offset, w, h)
    while offset + 2 < values.len() {
        let w = values[offset] as usize;
        let h = values[offset + 1] as usize;
        let pixel_count = w.checked_mul(h).unwrap_or(0);
        if w == 0 || h == 0 || offset + 2 + pixel_count > values.len() {
            break;
        }
        let data_start = offset + 2;
        match best {
            None => best = Some((data_start, w, h)),
            Some((_, bw, bh)) => {
                if w <= 48 && h <= 48 && w * h > bw * bh {
                    best = Some((data_start, w, h));
                } else if bw > 48 && w < bw {
                    best = Some((data_start, w, h));
                }
            }
        }
        offset += 2 + pixel_count;
    }

    let (data_start, src_w, src_h) = best?;
    let target = 32usize;
    let mut rgba = vec![0u8; target * target * 4];
    for y in 0..target {
        let src_y = (y * src_h) / target;
        for x in 0..target {
            let src_x = (x * src_w) / target;
            let argb = values[data_start + src_y * src_w + src_x];
            let a = ((argb >> 24) & 0xff) as u8;
            let r = ((argb >> 16) & 0xff) as u8;
            let g = ((argb >> 8) & 0xff) as u8;
            let b = (argb & 0xff) as u8;
            let dst = (y * target + x) * 4;
            // gpui expects BGRA byte order
            rgba[dst] = b;
            rgba[dst + 1] = g;
            rgba[dst + 2] = r;
            rgba[dst + 3] = a;
        }
    }

    Some(RgbaImage {
        data: rgba,
        width: target,
        height: target,
    })
}

fn x11_data_to_rgba(
    data: &[u8],
    src_w: usize,
    src_h: usize,
    channel_order: ChannelOrder,
) -> Option<Vec<u8>> {
    if src_w == 0 || src_h == 0 {
        return None;
    }
    let pixels = src_w.checked_mul(src_h)?;
    if pixels == 0 {
        return None;
    }
    let bytes_per_pixel = data.len() / pixels;
    if bytes_per_pixel < 3
        || channel_order.red >= bytes_per_pixel
        || channel_order.green >= bytes_per_pixel
        || channel_order.blue >= bytes_per_pixel
    {
        return None;
    }

    let mut rgba = Vec::with_capacity(pixels * 4);
    for i in 0..pixels {
        let src_base = i * bytes_per_pixel;
        let r = data[src_base + channel_order.red];
        let g = data[src_base + channel_order.green];
        let b = data[src_base + channel_order.blue];
        rgba.extend_from_slice(&[r, g, b, 255]);
    }
    Some(rgba)
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
