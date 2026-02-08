use gpui::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct ActiveMonitor {
    bounds: Bounds<Pixels>,
}

impl ActiveMonitor {
    pub fn centered_bounds(&self, win_size: Size<Pixels>) -> Bounds<Pixels> {
        let x = self.bounds.origin.x + (self.bounds.size.width - win_size.width) / 2.0;
        let y = self.bounds.origin.y + (self.bounds.size.height - win_size.height) / 3.0;
        Bounds::new(point(x, y), win_size)
    }

    pub fn bounds(&self) -> &Bounds<Pixels> {
        &self.bounds
    }
}

#[derive(Clone)]
pub struct FocusCache {
    cached: Arc<Mutex<Option<ActiveMonitor>>>,
    monitors: Vec<Bounds<Pixels>>,
}

impl FocusCache {
    pub fn start(cx: &App) -> Self {
        let monitors = physical_monitors(cx);
        let cached: Arc<Mutex<Option<ActiveMonitor>>> = Arc::new(Mutex::new(None));

        #[cfg(target_os = "linux")]
        {
            if !is_wayland() {
                let shared = cached.clone();
                let monitors_clone = monitors.clone();
                std::thread::spawn(move || {
                    x11_focus_listener(shared, monitors_clone);
                });
            }
        }

        #[cfg(target_os = "macos")]
        {
            let initial = poll_focus_once()
                .as_ref()
                .and_then(|snap| monitor_for_bounds(&monitors, &snap.bounds))
                .map(|bounds| ActiveMonitor { bounds });
            *cached.lock().unwrap() = initial;
        }

        Self { cached, monitors }
    }

    pub fn snapshot(&self) -> Option<ActiveMonitor> {
        if self.monitors.is_empty() {
            return None;
        }
        if self.monitors.len() == 1 {
            return Some(ActiveMonitor { bounds: self.monitors[0] });
        }

        self.cached
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .or_else(|| Some(ActiveMonitor { bounds: self.monitors[0] }))
    }
}

#[cfg(target_os = "linux")]
fn x11_focus_listener(cached: Arc<Mutex<Option<ActiveMonitor>>>, monitors: Vec<Bounds<Pixels>>) {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::*;

    let Ok((conn, screen_num)) = x11rb::connect(None) else { return };
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    let net_active_window = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| reply.atom);

    let wm_pid = conn
        .intern_atom(false, b"_NET_WM_PID")
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| reply.atom);

    let Some(atom) = net_active_window else { return };
    let own_pid = std::process::id();

    let update = |conn: &x11rb::rust_connection::RustConnection| {
        let result = resolve_focused_window(conn, root, atom, wm_pid, own_pid, &monitors);
        #[cfg(debug_assertions)]
        eprintln!("[monitor] resolve: {:?}, monitors: {:?}", result.as_ref().map(|m| &m.bounds), monitors);
        let Some(active_monitor) = result else { return };
        if let Ok(mut guard) = cached.lock() {
            *guard = Some(active_monitor);
        }
    };

    update(&conn);

    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .ok();
    conn.flush().ok();

    #[cfg(debug_assertions)]
    eprintln!("[monitor] listener started");

    loop {
        let Ok(event) = conn.wait_for_event() else {
            #[cfg(debug_assertions)]
            eprintln!("[monitor] event loop broke");
            break;
        };

        let x11rb::protocol::Event::PropertyNotify(ev) = event else { continue };
        if ev.atom != atom {
            continue;
        }

        #[cfg(debug_assertions)]
        eprintln!("[monitor] focus changed");
        update(&conn);
    }
}

#[cfg(target_os = "linux")]
fn resolve_focused_window(
    conn: &impl x11rb::connection::Connection,
    root: u32,
    active_window_atom: u32,
    wm_pid_atom: Option<u32>,
    own_pid: u32,
    monitors: &[Bounds<Pixels>],
) -> Option<ActiveMonitor> {
    use x11rb::protocol::xproto::*;

    let prop = conn
        .get_property(false, root, active_window_atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;

    let window_id = prop.value32()?.next()?;
    if window_id == 0 {
        return None;
    }

    if let Some(pid_atom) = wm_pid_atom {
        let pid_prop = conn
            .get_property(false, window_id, pid_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok());
        if let Some(pp) = pid_prop {
            if pp.value32().and_then(|mut v| v.next()) == Some(own_pid) {
                return None;
            }
        }
    }

    let geom = conn.get_geometry(window_id).ok()?.reply().ok()?;
    let coords = conn
        .translate_coordinates(window_id, root, 0, 0)
        .ok()?
        .reply()
        .ok()?;

    let bounds = Bounds::new(
        point(px(coords.dst_x as f32), px(coords.dst_y as f32)),
        size(px(geom.width as f32), px(geom.height as f32)),
    );

    let monitor_bounds = monitor_for_bounds(monitors, &bounds).unwrap_or(monitors[0]);
    Some(ActiveMonitor { bounds: monitor_bounds })
}

fn physical_monitors(cx: &App) -> Vec<Bounds<Pixels>> {
    let gpui_displays = cx.displays();
    if gpui_displays.len() > 1 {
        return gpui_displays.iter().map(|d| d.bounds()).collect();
    }

    #[cfg(target_os = "linux")]
    {
        let xrandr = xrandr_monitors();
        if xrandr.len() > 1 {
            return xrandr;
        }
    }

    #[cfg(target_os = "macos")]
    {
        let cg = macos_display_bounds();
        if cg.len() > 1 {
            return cg;
        }
    }

    gpui_displays.iter().map(|d| d.bounds()).collect()
}

fn monitor_for_bounds(monitors: &[Bounds<Pixels>], window: &Bounds<Pixels>) -> Option<Bounds<Pixels>> {
    monitors
        .iter()
        .filter_map(|m| {
            let area = intersection_area(window, m);
            if area > 0.0 { Some((*m, area)) } else { None }
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(m, _)| m)
}

fn intersection_area(a: &Bounds<Pixels>, b: &Bounds<Pixels>) -> f64 {
    let inter = a.intersect(b);
    if inter.size.width <= px(0.) || inter.size.height <= px(0.) {
        return 0.0;
    }
    inter.size.width.to_f64() * inter.size.height.to_f64()
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
fn parse_xrandr_line(line: &str) -> Option<Bounds<Pixels>> {
    if !line.contains(" connected") {
        return None;
    }

    let geom = line.split_whitespace().find(|s| s.contains('+') && s.contains('x'))?;
    let (res, offsets) = geom.split_once('+')?;
    let (w, h) = res.split_once('x')?;
    let (ox, oy) = offsets.split_once('+')?;

    Some(Bounds::new(
        point(px(ox.parse::<f32>().ok()?), px(oy.parse::<f32>().ok()?)),
        size(px(w.parse::<f32>().ok()?), px(h.parse::<f32>().ok()?)),
    ))
}

#[cfg(target_os = "linux")]
fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").map(|v| v == "wayland").unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(target_os = "macos")]
fn poll_focus_once() -> Option<FocusSnapshot> {
    use std::process::Command;

    let script = r#"
        tell application "System Events"
            set frontApp to first application process whose frontmost is true
            try
                set frontWindow to first window of frontApp
                set {x, y} to position of frontWindow
                set {w, h} to size of frontWindow
                return (x as string) & "," & (y as string) & "," & (w as string) & "," & (h as string)
            on error
                return "none"
            end try
        end tell
    "#;

    let out = Command::new("osascript")
        .args(["-e", script])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if stdout == "none" || stdout.is_empty() {
        return None;
    }

    let parts: Vec<i32> = stdout.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if parts.len() != 4 || parts[2] <= 0 || parts[3] <= 0 {
        return None;
    }

    Some(FocusSnapshot {
        bounds: Bounds::new(
            point(px(parts[0] as f32), px(parts[1] as f32)),
            size(px(parts[2] as f32), px(parts[3] as f32)),
        ),
    })
}

#[cfg(target_os = "macos")]
struct FocusSnapshot {
    bounds: Bounds<Pixels>,
}

#[cfg(target_os = "macos")]
fn macos_display_bounds() -> Vec<Bounds<Pixels>> {
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGRect { origin: CGPoint, size: CGSize }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGPoint { x: f64, y: f64 }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGSize { width: f64, height: f64 }

    type CGDirectDisplayID = u32;

    extern "C" {
        fn CGGetActiveDisplayList(max: u32, displays: *mut CGDirectDisplayID, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    }

    let mut ids = [0u32; 16];
    let mut count = 0u32;

    let ret = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) };
    if ret != 0 {
        return Vec::new();
    }

    (0..count as usize)
        .map(|i| {
            let rect = unsafe { CGDisplayBounds(ids[i]) };
            Bounds::new(
                point(px(rect.origin.x as f32), px(rect.origin.y as f32)),
                size(px(rect.size.width as f32), px(rect.size.height as f32)),
            )
        })
        .collect()
}
