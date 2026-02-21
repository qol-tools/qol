use super::PlatformQueries;
use gpui::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

pub(super) struct LinuxQueries {
    conn: Option<RustConnection>,
    root: u32,
    active_window_atom: u32,
    wm_pid_atom: Option<u32>,
    own_pid: u32,
}

impl LinuxQueries {
    pub fn new() -> Self {
        if is_wayland() {
            return Self {
                conn: None,
                root: 0,
                active_window_atom: 0,
                wm_pid_atom: None,
                own_pid: 0,
            };
        }

        let Ok((conn, screen_num)) = x11rb::connect(None) else {
            return Self {
                conn: None,
                root: 0,
                active_window_atom: 0,
                wm_pid_atom: None,
                own_pid: 0,
            };
        };

        let root = conn.setup().roots[screen_num].root;

        let active_window_atom = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.atom)
            .unwrap_or(0);

        let wm_pid_atom = conn
            .intern_atom(false, b"_NET_WM_PID")
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.atom);

        Self {
            conn: Some(conn),
            root,
            active_window_atom,
            wm_pid_atom,
            own_pid: std::process::id(),
        }
    }
}

impl PlatformQueries for LinuxQueries {
    fn cursor_position(&self) -> Option<(f32, f32)> {
        let conn = self.conn.as_ref()?;
        let pointer = conn.query_pointer(self.root).ok()?.reply().ok()?;
        Some((pointer.root_x as f32, pointer.root_y as f32))
    }

    fn focused_window_bounds(&self) -> Option<Bounds<Pixels>> {
        let conn = self.conn.as_ref()?;

        let prop = conn
            .get_property(
                false,
                self.root,
                self.active_window_atom,
                AtomEnum::WINDOW,
                0,
                1,
            )
            .ok()?
            .reply()
            .ok()?;

        let window_id = prop.value32()?.next()?;
        if window_id == 0 {
            return None;
        }

        if let Some(pid_atom) = self.wm_pid_atom {
            let pid_prop = conn
                .get_property(false, window_id, pid_atom, AtomEnum::CARDINAL, 0, 1)
                .ok()
                .and_then(|c| c.reply().ok());
            if let Some(pp) = pid_prop {
                if pp.value32().and_then(|mut v| v.next()) == Some(self.own_pid) {
                    return None;
                }
            }
        }

        let geom = conn.get_geometry(window_id).ok()?.reply().ok()?;
        let coords = conn
            .translate_coordinates(window_id, self.root, 0, 0)
            .ok()?
            .reply()
            .ok()?;

        Some(Bounds::new(
            point(px(coords.dst_x as f32), px(coords.dst_y as f32)),
            size(px(geom.width as f32), px(geom.height as f32)),
        ))
    }

    fn physical_monitors(&self) -> Vec<Bounds<Pixels>> {
        xrandr_monitors()
    }
}

fn xrandr_monitors() -> Vec<Bounds<Pixels>> {
    use std::process::Command;

    let out = match Command::new("xrandr").arg("--current").output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(parse_xrandr_line)
        .collect()
}

fn parse_xrandr_line(line: &str) -> Option<Bounds<Pixels>> {
    if !line.contains(" connected") {
        return None;
    }

    let geom = line
        .split_whitespace()
        .find(|s| s.contains('+') && s.contains('x'))?;
    let (res, offsets) = geom.split_once('+')?;
    let (w, h) = res.split_once('x')?;
    let (ox, oy) = offsets.split_once('+')?;

    Some(Bounds::new(
        point(px(ox.parse::<f32>().ok()?), px(oy.parse::<f32>().ok()?)),
        size(px(w.parse::<f32>().ok()?), px(h.parse::<f32>().ok()?)),
    ))
}

fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}
