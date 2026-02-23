use super::preview;
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
    let rgba = x11_data_to_rgba(
        &image.data,
        geometry.width as usize,
        geometry.height as usize,
        channel_order,
    )?;
    preview::downscale_and_save_preview(
        window_id,
        &rgba,
        geometry.width as usize,
        geometry.height as usize,
        max_w,
        max_h,
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

        let rgba = x11_data_to_rgba(
            &reply.data,
            geo.width as usize,
            geo.height as usize,
            channel_order,
        );
        let path = rgba.and_then(|rgba| {
            preview::downscale_and_save_preview(
                window_id,
                &rgba,
                geo.width as usize,
                geo.height as usize,
                max_w,
                max_h,
            )
        });
        out.push((list_index, path));
    }

    out
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
