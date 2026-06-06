use crate::desktop_state::Platform;
use qol_runtime::MonitorBounds;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

pub(super) struct LinuxQueries {
    conn: Option<RustConnection>,
    root: u32,
    active_window_atom: u32,
    client_list_stacking_atom: u32,
    client_list_atom: u32,
    wm_pid_atom: Option<u32>,
    own_pid: u32,
}

impl LinuxQueries {
    pub(super) fn new() -> Self {
        if is_wayland() {
            return Self::disconnected();
        }
        let Ok((conn, screen_num)) = x11rb::connect(None) else {
            return Self::disconnected();
        };
        Self::from_x11_conn(conn, screen_num)
    }

    fn disconnected() -> Self {
        Self {
            conn: None,
            root: 0,
            active_window_atom: 0,
            client_list_stacking_atom: 0,
            client_list_atom: 0,
            wm_pid_atom: None,
            own_pid: 0,
        }
    }

    fn from_x11_conn(conn: RustConnection, screen_num: usize) -> Self {
        let root = conn.setup().roots[screen_num].root;
        let active_window_atom = intern_atom_id(&conn, b"_NET_ACTIVE_WINDOW").unwrap_or(0);
        let client_list_stacking_atom =
            intern_atom_id(&conn, b"_NET_CLIENT_LIST_STACKING").unwrap_or(0);
        let client_list_atom = intern_atom_id(&conn, b"_NET_CLIENT_LIST").unwrap_or(0);
        let wm_pid_atom = intern_atom_id(&conn, b"_NET_WM_PID");
        Self {
            conn: Some(conn),
            root,
            active_window_atom,
            client_list_stacking_atom,
            client_list_atom,
            wm_pid_atom,
            own_pid: std::process::id(),
        }
    }
}

impl Platform for LinuxQueries {
    fn cursor_position(&self) -> Option<(f32, f32)> {
        let conn = self.conn.as_ref()?;
        let pointer = conn.query_pointer(self.root).ok()?.reply().ok()?;
        Some((pointer.root_x as f32, pointer.root_y as f32))
    }

    fn focused_window_bounds(&self) -> Option<MonitorBounds> {
        let conn = self.conn.as_ref()?;
        let window_id = get_active_window_id(conn, self.root, self.active_window_atom)?;
        let pid = window_pid(conn, window_id, self.wm_pid_atom);
        let ignored = pid.is_some_and(|pid| is_self_or_ignored(pid, self.own_pid));
        let bounds = if ignored {
            None
        } else {
            get_window_bounds(conn, self.root, window_id)
        };
        #[cfg(debug_assertions)]
        log_focus_change(conn, window_id, pid, ignored, bounds.as_ref());
        if ignored {
            return None;
        }
        bounds
    }

    fn physical_monitors(&self) -> Vec<MonitorBounds> {
        xrandr_monitors()
    }

    fn window_list_fingerprint(&self) -> Option<u64> {
        let conn = self.conn.as_ref()?;
        let list_atom = if self.client_list_stacking_atom != 0 {
            self.client_list_stacking_atom
        } else {
            self.client_list_atom
        };
        if list_atom == 0 {
            return None;
        }
        let prop = conn
            .get_property(false, self.root, list_atom, AtomEnum::WINDOW, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        let value32 = prop.value32()?;
        let ids: Vec<u32> = value32.collect();
        if ids.is_empty() {
            return None;
        }
        let active_window =
            get_active_window_id(conn, self.root, self.active_window_atom).unwrap_or(0);
        let mut h: u64 = 0;
        for id in ids {
            let mut x = (id as u64).wrapping_mul(0x9E3779B97F4A7C15);
            x ^= x >> 32;
            h ^= x;
        }
        let mut x = (active_window as u64).wrapping_mul(0x9E3779B97F4A7C15);
        x ^= x >> 32;
        h ^= x;
        Some(h)
    }
}

fn intern_atom_id(conn: &RustConnection, name: &[u8]) -> Option<u32> {
    conn.intern_atom(false, name)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
}

fn get_active_window_id(conn: &RustConnection, root: u32, active_window_atom: u32) -> Option<u32> {
    let prop = conn
        .get_property(false, root, active_window_atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let window_id = prop.value32()?.next()?;
    if window_id == 0 {
        return None;
    }
    Some(window_id)
}

fn window_pid(conn: &RustConnection, window_id: u32, wm_pid_atom: Option<u32>) -> Option<u32> {
    let pid_atom = wm_pid_atom?;
    let pid_prop = conn
        .get_property(false, window_id, pid_atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    pid_prop.value32().and_then(|mut v| v.next())
}

fn is_self_or_ignored(pid: u32, own_pid: u32) -> bool {
    pid == own_pid || super::super::is_ignored_pid(pid)
}

#[cfg(debug_assertions)]
static LAST_FOCUS_WID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(debug_assertions)]
fn log_focus_change(
    conn: &RustConnection,
    window_id: u32,
    pid: Option<u32>,
    ignored: bool,
    bounds: Option<&MonitorBounds>,
) {
    use std::sync::atomic::Ordering;
    if LAST_FOCUS_WID.swap(window_id, Ordering::Relaxed) == window_id {
        return;
    }
    let pid_str = pid
        .map(|p| p.to_string())
        .unwrap_or_else(|| "?".to_string());
    let winpos = bounds
        .map(|b| {
            format!(
                "({},{},{}x{})",
                b.x as i32, b.y as i32, b.width as i32, b.height as i32
            )
        })
        .unwrap_or_else(|| "none".to_string());
    let title = focus_window_title(conn, window_id).unwrap_or_default();
    qol_runtime::probe!(
        "FOCUS_WIN",
        "wid={window_id} pid={pid_str} ignored={ignored} winpos={winpos} title={title:?}"
    );
}

#[cfg(debug_assertions)]
fn focus_window_title(conn: &RustConnection, window_id: u32) -> Option<String> {
    let read = |atom: u32, ty: u32| -> Option<String> {
        let reply = conn
            .get_property(false, window_id, atom, ty, 0, 256)
            .ok()?
            .reply()
            .ok()?;
        if reply.value.is_empty() {
            return None;
        }
        Some(
            String::from_utf8_lossy(&reply.value)
                .trim_end_matches('\0')
                .to_string(),
        )
    };
    let net_wm_name = intern_atom_id(conn, b"_NET_WM_NAME");
    let utf8 = intern_atom_id(conn, b"UTF8_STRING");
    net_wm_name
        .zip(utf8)
        .and_then(|(atom, ty)| read(atom, ty))
        .or_else(|| read(AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()))
}

fn get_window_bounds(conn: &RustConnection, root: u32, window_id: u32) -> Option<MonitorBounds> {
    let geom = conn.get_geometry(window_id).ok()?.reply().ok()?;
    let coords = conn
        .translate_coordinates(window_id, root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    Some(MonitorBounds {
        x: coords.dst_x as f32,
        y: coords.dst_y as f32,
        width: geom.width as f32,
        height: geom.height as f32,
    })
}

fn xrandr_monitors() -> Vec<MonitorBounds> {
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

fn parse_xrandr_line(line: &str) -> Option<MonitorBounds> {
    if !line.contains(" connected") {
        return None;
    }

    let geom = line
        .split_whitespace()
        .find(|s| s.contains('+') && s.contains('x'))?;
    let (res, offsets) = geom.split_once('+')?;
    let (w, h) = res.split_once('x')?;
    let (ox, oy) = offsets.split_once('+')?;

    Some(MonitorBounds {
        x: ox.parse::<f32>().ok()?,
        y: oy.parse::<f32>().ok()?,
        width: w.parse::<f32>().ok()?,
        height: h.parse::<f32>().ok()?,
    })
}

fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(test)]
mod tests {
    use super::is_self_or_ignored;
    use crate::desktop_state::{add_ignore_pid, remove_ignore_pid};

    #[test]
    fn skips_own_pid_and_registered_daemon_pids_only() {
        let own_pid = 424_242;
        let daemon_pid = 535_353;
        add_ignore_pid(daemon_pid);
        let cases = [
            (own_pid, true, "own pid is always skipped"),
            (daemon_pid, true, "registered plugin-daemon pid is skipped"),
            (
                999_999,
                false,
                "unrelated foreign window pid is not skipped",
            ),
        ];
        for (pid, expected, label) in cases {
            assert_eq!(is_self_or_ignored(pid, own_pid), expected, "{label}");
        }
        remove_ignore_pid(daemon_pid);
    }
}
