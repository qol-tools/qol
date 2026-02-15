use super::{
    monitor_for_bounds, monitor_for_point, promote_pending_cursor, track_cursor_monitor,
    ActiveMonitor, InputState, Stamped,
};
use gpui::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const POINTER_POLL_MS: u64 = 50;
const ALT_TAB_GRACE_MS: u64 = 700;
const POST_LAUNCHER_FOCUS_GUARD_MS: u64 = 1200;
const EVENT_LOOP_IDLE_MS: u64 = 10;

pub(super) fn start_focus_tracking(state: Arc<Mutex<InputState>>, monitors: Vec<Bounds<Pixels>>) {
    if !is_wayland() {
        std::thread::spawn(move || {
            x11_focus_listener(state, monitors);
        });
    }
}

pub(super) fn xrandr_monitors() -> Vec<Bounds<Pixels>> {
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

fn x11_focus_listener(state: Arc<Mutex<InputState>>, monitors: Vec<Bounds<Pixels>>) {
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
        let masks = [xinput::EventMask {
            deviceid: 1,
            mask: vec![input_mask],
        }];
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
                        last_alt_tab_at = None;
                        #[cfg(debug_assertions)]
                        eprintln!("[monitor] launcher active; focus guard enabled");
                        continue;
                    };

                    if post_launcher_guard_until.is_some_and(|until| now >= until) {
                        post_launcher_started_at = None;
                        post_launcher_guard_until = None;
                        last_alt_tab_at = None;
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

fn query_pointer_monitor(
    conn: &impl x11rb::connection::Connection,
    root: u32,
    monitors: &[Bounds<Pixels>],
) -> Option<Bounds<Pixels>> {
    use x11rb::protocol::xproto::ConnectionExt as _;

    let pointer = conn.query_pointer(root).ok()?.reply().ok()?;
    monitor_for_point(monitors, pointer.root_x as f32, pointer.root_y as f32)
}

fn alt_tab_keycodes(
    conn: &impl x11rb::connection::Connection,
) -> (HashSet<u32>, HashSet<u32>) {
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
        return (HashSet::new(), HashSet::new());
    };

    let per = reply.keysyms_per_keycode as usize;
    if per == 0 {
        return (HashSet::new(), HashSet::new());
    }

    let mut alt = HashSet::new();
    let mut tab = HashSet::new();

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

fn is_any_keycode_down(
    conn: &impl x11rb::connection::Connection,
    keycodes: &HashSet<u32>,
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
