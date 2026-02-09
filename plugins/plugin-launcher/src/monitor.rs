use gpui::*;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SETTLE_MS: u64 = 400;
#[cfg(target_os = "linux")]
const POINTER_POLL_MS: u64 = 50;
#[cfg(target_os = "linux")]
const ALT_TAB_GRACE_MS: u64 = 700;
#[cfg(target_os = "linux")]
const POST_LAUNCHER_FOCUS_GUARD_MS: u64 = 1200;
#[cfg(target_os = "linux")]
const EVENT_LOOP_IDLE_MS: u64 = 10;

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

#[derive(Clone, Debug)]
struct Stamped {
    monitor: ActiveMonitor,
    at: Instant,
}

#[derive(Clone, Debug, Default)]
struct InputState {
    focus: Option<Stamped>,
    cursor: Option<Stamped>,
    pending_cursor: Option<Stamped>,
}

fn monitor_for_point(monitors: &[Bounds<Pixels>], x: f32, y: f32) -> Option<Bounds<Pixels>> {
    monitors
        .iter()
        .find(|m| {
            let right = m.origin.x + m.size.width;
            let bottom = m.origin.y + m.size.height;
            px(x) >= m.origin.x && px(x) < right && px(y) >= m.origin.y && px(y) < bottom
        })
        .copied()
}

fn track_cursor_monitor(state: &mut InputState, monitor: Bounds<Pixels>, at: Instant) {
    let same_settled = state
        .cursor
        .as_ref()
        .is_some_and(|c| c.monitor.bounds == monitor);
    if same_settled {
        state.pending_cursor = None;
        return;
    }

    let same_pending = state
        .pending_cursor
        .as_ref()
        .is_some_and(|p| p.monitor.bounds == monitor);
    if same_pending {
        return;
    }

    state.pending_cursor = Some(Stamped {
        monitor: ActiveMonitor { bounds: monitor },
        at,
    });
}

fn promote_pending_cursor(state: &mut InputState, now: Instant) {
    if let Some(ref pending) = state.pending_cursor {
        if now.duration_since(pending.at).as_millis() >= SETTLE_MS as u128 {
            state.cursor = state.pending_cursor.take();
        }
    }
}

fn pick_active_monitor(state: &InputState, fallback: Bounds<Pixels>) -> ActiveMonitor {
    match (state.cursor.as_ref(), state.focus.as_ref()) {
        (Some(cursor), Some(focus)) => {
            if cursor.at >= focus.at {
                cursor.monitor.clone()
            } else {
                focus.monitor.clone()
            }
        }
        (Some(cursor), None) => cursor.monitor.clone(),
        (None, Some(focus)) => focus.monitor.clone(),
        (None, None) => ActiveMonitor { bounds: fallback },
    }
}

#[derive(Clone)]
pub struct FocusCache {
    state: Arc<Mutex<InputState>>,
    monitors: Vec<Bounds<Pixels>>,
}

impl FocusCache {
    pub fn start(cx: &App) -> Self {
        let monitors = physical_monitors(cx);
        let state = Arc::new(Mutex::new(InputState::default()));

        #[cfg(target_os = "linux")]
        {
            if !is_wayland() {
                let shared = state.clone();
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
                .map(|bounds| Stamped {
                    monitor: ActiveMonitor { bounds },
                    at: Instant::now(),
                });
            state.lock().unwrap().focus = initial;
        }

        Self { state, monitors }
    }

    pub fn snapshot(&self) -> Option<ActiveMonitor> {
        if self.monitors.is_empty() {
            return None;
        }
        if self.monitors.len() == 1 {
            return Some(ActiveMonitor {
                bounds: self.monitors[0],
            });
        }

        let mut guard = self.state.lock().ok()?;
        let now = Instant::now();

        #[cfg(target_os = "linux")]
        if !is_wayland() {
            if let Some(pointer_monitor) = query_pointer_monitor_once(&self.monitors) {
                track_cursor_monitor(&mut guard, pointer_monitor, now);
            }
        }
        promote_pending_cursor(&mut guard, now);

        let result = pick_active_monitor(&guard, self.monitors[0]);

        #[cfg(debug_assertions)]
        eprintln!(
            "[monitor] snapshot: cursor={:?}, pending={:?}, focus={:?}",
            guard.cursor.as_ref().map(|c| &c.monitor.bounds),
            guard.pending_cursor.as_ref().map(|p| &p.monitor.bounds),
            guard.focus.as_ref().map(|f| &f.monitor.bounds),
        );

        Some(result)
    }
}

#[cfg(target_os = "linux")]
fn x11_focus_listener(state: Arc<Mutex<InputState>>, monitors: Vec<Bounds<Pixels>>) {
    use std::collections::HashSet;
    use x11rb::connection::Connection;
    use x11rb::protocol::xinput;
    use x11rb::protocol::xproto::*;

    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return;
    };
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

    let Some(atom) = net_active_window else {
        return;
    };
    let own_pid = std::process::id();
    let mut post_launcher_started_at: Option<Instant> = None;
    let mut post_launcher_guard_until: Option<Instant> = None;
    let mut last_pointer_poll_at = Instant::now() - Duration::from_millis(POINTER_POLL_MS);
    let (alt_keycodes, tab_keycodes) = alt_tab_keycodes(&conn);
    let mut alt_keys_down: HashSet<u32> = HashSet::new();
    let mut last_alt_tab_at: Option<Instant> = None;

    let resolve = |conn: &x11rb::rust_connection::RustConnection| {
        let result = resolve_focused_window(conn, root, atom, wm_pid, own_pid, &monitors);
        #[cfg(debug_assertions)]
        eprintln!(
            "[monitor] resolve: {:?}",
            result.as_ref().map(|m| &m.bounds)
        );
        result
    };

    if let Some(active_monitor) = resolve(&conn) {
        if let Ok(mut guard) = state.lock() {
            guard.focus = Some(Stamped {
                monitor: active_monitor,
                at: Instant::now(),
            });
        }
    }
    if let Some(pointer_monitor) = query_pointer_monitor(&conn, root, &monitors) {
        let now = Instant::now();
        if let Ok(mut guard) = state.lock() {
            track_cursor_monitor(&mut guard, pointer_monitor, now);
            promote_pending_cursor(&mut guard, now);
        }
    }

    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .ok();

    if xinput::xi_query_version(&conn, 2, 2)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some()
    {
        let key_mask = xinput::XIEventMask::RAW_KEY_PRESS | xinput::XIEventMask::RAW_KEY_RELEASE;
        let click_mask = xinput::XIEventMask::RAW_BUTTON_PRESS;
        let input_mask = key_mask | click_mask;
        let masks = [
            xinput::EventMask {
                // XIAllMasterDevices
                deviceid: 1,
                mask: vec![input_mask],
            },
            xinput::EventMask {
                // XIAllDevices
                deviceid: 0,
                mask: vec![input_mask],
            },
        ];
        xinput::xi_select_events(&conn, root, &masks).ok();
        #[cfg(debug_assertions)]
        eprintln!("[monitor] xinput2 initialized");
    } else {
        #[cfg(debug_assertions)]
        eprintln!("[monitor] xinput2 not available");
    }

    conn.flush().ok();

    #[cfg(debug_assertions)]
    eprintln!("[monitor] listener started");

    loop {
        match conn.poll_for_event() {
            Ok(Some(event)) => match event {
                x11rb::protocol::Event::PropertyNotify(ev) => {
                    if ev.atom != atom {
                        continue;
                    }

                    let now = Instant::now();
                    let Some(active_monitor) = resolve(&conn) else {
                        post_launcher_started_at = Some(now);
                        post_launcher_guard_until =
                            Some(now + Duration::from_millis(POST_LAUNCHER_FOCUS_GUARD_MS));
                        #[cfg(debug_assertions)]
                        eprintln!("[monitor] launcher active; focus guard enabled");
                        continue;
                    };

                    if post_launcher_guard_until.is_some_and(|until| now >= until) {
                        post_launcher_started_at = None;
                        post_launcher_guard_until = None;
                    }

                    let alt_tab_recent = last_alt_tab_at.is_some_and(|at| {
                        let started_after_launcher =
                            post_launcher_started_at.is_some_and(|start| at >= start);
                        started_after_launcher
                            && now.duration_since(at).as_millis() <= ALT_TAB_GRACE_MS as u128
                    });
                    let alt_held_now = is_any_keycode_down(&conn, &alt_keycodes);

                    if post_launcher_guard_until.is_some_and(|until| now < until)
                        && !alt_tab_recent
                        && !alt_held_now
                    {
                        #[cfg(debug_assertions)]
                        eprintln!("[monitor] suppressed post-launcher focus (guard)");
                        continue;
                    }
                    #[cfg(debug_assertions)]
                    if post_launcher_guard_until.is_some_and(|until| now < until)
                        && (alt_tab_recent || alt_held_now)
                    {
                        eprintln!(
                            "[monitor] allowing guarded focus (alt_tab_recent={}, alt_held_now={})",
                            alt_tab_recent, alt_held_now
                        );
                    }

                    post_launcher_started_at = None;
                    post_launcher_guard_until = None;
                    #[cfg(debug_assertions)]
                    eprintln!("[monitor] focus → {:?}", active_monitor.bounds);
                    if let Ok(mut guard) = state.lock() {
                        guard.focus = Some(Stamped {
                            monitor: active_monitor,
                            at: now,
                        });
                    }
                }
                x11rb::protocol::Event::XinputRawKeyPress(ev) => {
                    let keycode = ev.detail;
                    if alt_keycodes.contains(&keycode) {
                        alt_keys_down.insert(keycode);
                    }
                    if tab_keycodes.contains(&keycode) && !alt_keys_down.is_empty() {
                        last_alt_tab_at = Some(Instant::now());
                        #[cfg(debug_assertions)]
                        eprintln!("[monitor] detected alt-tab");
                    }
                }
                x11rb::protocol::Event::XinputRawKeyRelease(ev) => {
                    let keycode = ev.detail;
                    if alt_keycodes.contains(&keycode) {
                        alt_keys_down.remove(&keycode);
                    }
                }
                x11rb::protocol::Event::XinputRawButtonPress(_) => {
                    let now = Instant::now();
                    if let Some(pointer_monitor) = query_pointer_monitor(&conn, root, &monitors) {
                        if let Ok(mut guard) = state.lock() {
                            #[cfg(debug_assertions)]
                            eprintln!("[monitor] cursor click → {:?}", pointer_monitor);
                            guard.cursor = Some(Stamped {
                                monitor: ActiveMonitor {
                                    bounds: pointer_monitor,
                                },
                                at: now,
                            });
                            guard.pending_cursor = None;
                        }
                    }
                }
                _ => {}
            },
            Ok(None) => {
                let now = Instant::now();

                if post_launcher_guard_until.is_some_and(|until| now >= until) {
                    post_launcher_started_at = None;
                    post_launcher_guard_until = None;
                    #[cfg(debug_assertions)]
                    eprintln!("[monitor] focus guard expired");
                }

                if now.duration_since(last_pointer_poll_at).as_millis() >= POINTER_POLL_MS as u128 {
                    if let Some(pointer_monitor) = query_pointer_monitor(&conn, root, &monitors) {
                        if let Ok(mut guard) = state.lock() {
                            track_cursor_monitor(&mut guard, pointer_monitor, now);
                            promote_pending_cursor(&mut guard, now);
                        }
                    }
                    last_pointer_poll_at = now;
                }

                std::thread::sleep(Duration::from_millis(EVENT_LOOP_IDLE_MS));
            }
            Err(_) => {
                #[cfg(debug_assertions)]
                eprintln!("[monitor] event loop broke");
                break;
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn query_pointer_monitor(
    conn: &impl x11rb::connection::Connection,
    root: u32,
    monitors: &[Bounds<Pixels>],
) -> Option<Bounds<Pixels>> {
    use x11rb::protocol::xproto::ConnectionExt as _;

    let pointer = conn.query_pointer(root).ok()?.reply().ok()?;
    monitor_for_point(monitors, pointer.root_x as f32, pointer.root_y as f32)
}

#[cfg(target_os = "linux")]
fn query_pointer_monitor_once(monitors: &[Bounds<Pixels>]) -> Option<Bounds<Pixels>> {
    use x11rb::connection::Connection;

    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return None;
    };
    let root = conn.setup().roots.get(screen_num)?.root;
    query_pointer_monitor(&conn, root, monitors)
}

#[cfg(target_os = "linux")]
fn alt_tab_keycodes(
    conn: &impl x11rb::connection::Connection,
) -> (
    std::collections::HashSet<u32>,
    std::collections::HashSet<u32>,
) {
    use x11rb::protocol::xproto::ConnectionExt as _;

    const XK_TAB: u32 = 0xff09;
    const XK_ALT_L: u32 = 0xffe9;
    const XK_ALT_R: u32 = 0xffea;

    let setup = conn.setup();
    let min = setup.min_keycode;
    let max = setup.max_keycode;
    let keycode_count = max.saturating_sub(min).saturating_add(1);

    let Some(reply) = conn
        .get_keyboard_mapping(min, keycode_count)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return (
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
        );
    };

    let per = reply.keysyms_per_keycode as usize;
    if per == 0 {
        return (
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
        );
    }

    let mut alt = std::collections::HashSet::new();
    let mut tab = std::collections::HashSet::new();

    for (idx, keysyms) in reply.keysyms.chunks(per).enumerate() {
        let keycode = u32::from(min) + idx as u32;
        if keysyms.iter().any(|sym| *sym == XK_TAB) {
            tab.insert(keycode);
        }
        if keysyms
            .iter()
            .any(|sym| *sym == XK_ALT_L || *sym == XK_ALT_R)
        {
            alt.insert(keycode);
        }
    }

    (alt, tab)
}

#[cfg(target_os = "linux")]
fn is_any_keycode_down(
    conn: &impl x11rb::connection::Connection,
    keycodes: &std::collections::HashSet<u32>,
) -> bool {
    use x11rb::protocol::xproto::ConnectionExt as _;

    if keycodes.is_empty() {
        return false;
    }

    let Some(reply) = conn
        .query_keymap()
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return false;
    };

    keycodes.iter().any(|&keycode| {
        if keycode > 255 {
            return false;
        }
        let byte = (keycode >> 3) as usize;
        if byte >= reply.keys.len() {
            return false;
        }
        let bit = 1u8 << (keycode & 7);
        (reply.keys[byte] & bit) != 0
    })
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
    Some(ActiveMonitor {
        bounds: monitor_bounds,
    })
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

fn monitor_for_bounds(
    monitors: &[Bounds<Pixels>],
    window: &Bounds<Pixels>,
) -> Option<Bounds<Pixels>> {
    monitors
        .iter()
        .filter_map(|m| {
            let area = intersection_area(window, m);
            if area > 0.0 {
                Some((*m, area))
            } else {
                None
            }
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

#[cfg(target_os = "linux")]
fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
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

    let parts: Vec<i32> = stdout
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
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
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    type CGDirectDisplayID = u32;

    extern "C" {
        fn CGGetActiveDisplayList(
            max: u32,
            displays: *mut CGDirectDisplayID,
            count: *mut u32,
        ) -> i32;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn mon(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    fn stamped(bounds: Bounds<Pixels>, at: Instant) -> Stamped {
        Stamped {
            monitor: ActiveMonitor { bounds },
            at,
        }
    }

    #[::std::prelude::v1::test]
    fn monitor_for_point_finds_correct_monitor() {
        let monitors = vec![
            mon(0.0, 0.0, 1920.0, 1080.0),
            mon(1920.0, 0.0, 2560.0, 1440.0),
        ];
        let cases = [
            (100.0, 100.0, Some(monitors[0])),
            (1920.0, 500.0, Some(monitors[1])),
            (3000.0, 700.0, Some(monitors[1])),
            (5000.0, 0.0, None),
            (0.0, 1080.0, None),
        ];
        for (x, y, expected) in cases {
            assert_eq!(
                monitor_for_point(&monitors, x, y),
                expected,
                "point: ({x}, {y})"
            );
        }
    }

    #[::std::prelude::v1::test]
    fn monitor_for_point_at_origin() {
        let monitors = vec![
            mon(0.0, 0.0, 1920.0, 1080.0),
            mon(1920.0, 0.0, 2560.0, 1440.0),
        ];
        assert_eq!(monitor_for_point(&monitors, 0.0, 0.0), Some(monitors[0]));
        assert_eq!(monitor_for_point(&monitors, 1920.0, 0.0), Some(monitors[1]));
    }

    #[::std::prelude::v1::test]
    fn track_cursor_monitor_creates_pending_on_monitor_change() {
        let m_a = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_b = mon(1920.0, 0.0, 2560.0, 1440.0);
        let now = Instant::now();
        let mut state = InputState {
            focus: None,
            cursor: Some(stamped(m_a, now - Duration::from_secs(2))),
            pending_cursor: None,
        };

        track_cursor_monitor(&mut state, m_b, now);

        assert_eq!(
            state.pending_cursor.as_ref().map(|p| p.monitor.bounds),
            Some(m_b)
        );
    }

    #[::std::prelude::v1::test]
    fn snapshot_prefers_newer_cursor_over_focus() {
        let m_focus = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_cursor = mon(1920.0, 0.0, 2560.0, 1440.0);
        let now = Instant::now();
        let state = InputState {
            focus: Some(stamped(m_focus, now - Duration::from_secs(2))),
            cursor: Some(stamped(m_cursor, now - Duration::from_secs(1))),
            pending_cursor: None,
        };

        let result = snapshot_from(&state, &[m_focus, m_cursor]);
        assert_eq!(result.unwrap().bounds, m_cursor);
    }

    #[::std::prelude::v1::test]
    fn snapshot_prefers_newer_focus_over_cursor() {
        let m_focus = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_cursor = mon(1920.0, 0.0, 2560.0, 1440.0);
        let now = Instant::now();
        let state = InputState {
            focus: Some(stamped(m_focus, now)),
            cursor: Some(stamped(m_cursor, now - Duration::from_secs(3))),
            pending_cursor: None,
        };

        let result = snapshot_from(&state, &[m_focus, m_cursor]);
        assert_eq!(result.unwrap().bounds, m_focus);
    }

    #[::std::prelude::v1::test]
    fn snapshot_promotes_settled_pending() {
        let m = mon(1920.0, 0.0, 2560.0, 1440.0);
        let fallback = mon(0.0, 0.0, 1920.0, 1080.0);
        let mut state = InputState {
            focus: None,
            cursor: None,
            pending_cursor: Some(stamped(m, Instant::now() - Duration::from_millis(500))),
        };
        let result = snapshot_from_mut(&mut state, &[fallback, m]);
        assert_eq!(result.unwrap().bounds, m);
        assert!(state.cursor.is_some());
        assert!(state.pending_cursor.is_none());
    }

    #[::std::prelude::v1::test]
    fn snapshot_ignores_unsettled_pending() {
        let m_focus = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_pending = mon(1920.0, 0.0, 2560.0, 1440.0);
        let mut state = InputState {
            focus: Some(stamped(m_focus, Instant::now())),
            cursor: None,
            pending_cursor: Some(stamped(m_pending, Instant::now())),
        };
        let result = snapshot_from_mut(&mut state, &[m_focus, m_pending]);
        assert_eq!(result.unwrap().bounds, m_focus);
    }

    #[::std::prelude::v1::test]
    fn snapshot_returns_fallback_with_no_signals() {
        let fallback = mon(0.0, 0.0, 1920.0, 1080.0);
        let state = InputState::default();
        let result = snapshot_from(&state, &[fallback]);
        assert_eq!(result.unwrap().bounds, fallback);
    }

    fn snapshot_from(state: &InputState, monitors: &[Bounds<Pixels>]) -> Option<ActiveMonitor> {
        let mut state = state.clone();
        snapshot_from_mut(&mut state, monitors)
    }

    fn snapshot_from_mut(
        state: &mut InputState,
        monitors: &[Bounds<Pixels>],
    ) -> Option<ActiveMonitor> {
        let now = Instant::now();
        promote_pending_cursor(state, now);
        Some(pick_active_monitor(state, monitors[0]))
    }
}
