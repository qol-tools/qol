use gpui::{App, Bounds, Pixels};
use qol_sync::Mutex;
use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::Duration;

use crate::Rect;

mod app_audio;
mod clipboard;
mod display;
mod nvfbc;
mod preview;
mod preview_window;
mod recording;
mod system;
mod window;

pub use super::unix::{directory_is_read_only, owned_by_current_user, watch_daemon_stop};
pub use clipboard::{copy_image_to_clipboard, copy_path_to_clipboard};
pub use display::{full_screen_bounds, get_monitors};
pub use preview::{capture_frozen_frame, grab_preview_rgba};
pub(crate) use preview_window::{
    complete_pin_transition, prepare_preview_window, register_pin_transition,
};
pub use recording::{
    capture_screenshot, recording_format, recording_started, recording_stopped, start_capture,
    stop_capture,
};
use system::resolve_command;
pub use system::{
    external_services_check, list_audio_apps, list_audio_sinks, list_audio_sources,
    platform_supported_check, required_binaries_check,
};
pub use window::{
    configure_pin_window, pin_focus, pin_release_focus, pin_resize_session, prepare_pin_window,
    PinResizeSession,
};
pub(crate) use window::{configure_selector_window, prepare_selector_window};

const MIN_DETECTED_TARGET_PX: i32 = 24;

pub fn pin_cache_enabled() -> bool {
    true
}

static REVEAL_REQUESTED: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub(crate) fn mark_reveal_requested(title: &str) {
    {
        let mut requested = REVEAL_REQUESTED.lock();
        requested.insert(title.to_string());
    }
}

fn reveal_requested(title: &str) -> bool {
    REVEAL_REQUESTED.lock().contains(title)
}

pub(crate) fn reassert_parked(title: &str, cx: &mut App) {
    let title = title.to_string();
    cx.spawn(async move |cx| {
        for _ in 0..8 {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let parked = cx
                .update(|_| qol_gpui::popup_window::visible_windows_by_title_prefix(&title) == 0)
                .unwrap_or(true);
            if parked {
                return;
            }
            if reveal_requested(&title) {
                return;
            }
            let _ = cx.update(|_| qol_gpui::popup_window::hide_invisible(&title));
        }
    })
    .detach();
}

pub fn after_pin_open(title: &str) {
    qol_gpui::popup_window::hide_invisible(title);
}

pub fn run_internal_mode() -> Option<std::process::ExitCode> {
    recording::run_internal_capture_helper().or_else(app_audio::run_internal_helper)
}

pub(crate) fn detectable_window_rects(
    selector_bounds: Bounds<Pixels>,
    include_frame: bool,
) -> Vec<Rect> {
    x11_stacked_window_rects(include_frame)
        .into_iter()
        .filter(|rect| {
            rect.w >= MIN_DETECTED_TARGET_PX
                && rect.h >= MIN_DETECTED_TARGET_PX
                && rect_intersects_bounds(*rect, selector_bounds)
        })
        .collect()
}

pub(crate) fn runtime_monitor_rects() -> Vec<Rect> {
    let Some(state) = qol_runtime::PlatformStateClient::from_env().get_state() else {
        return Vec::new();
    };
    state
        .monitors
        .iter()
        .map(|monitor| Rect {
            x: monitor.x.round() as i32,
            y: monitor.y.round() as i32,
            w: monitor.width.round() as i32,
            h: monitor.height.round() as i32,
        })
        .collect()
}

fn x11_stacked_window_rects(include_frame: bool) -> Vec<Rect> {
    x11_stacked_windows(include_frame)
        .unwrap_or_default()
        .into_iter()
        .map(|(_, rect)| rect)
        .collect()
}

fn window_process_at(x: i32, y: i32) -> Option<u32> {
    let (window, _) = x11_stacked_windows(false)?.into_iter().find(|(_, rect)| {
        x >= rect.x && y >= rect.y && x < rect.x + rect.w && y < rect.y + rect.h
    })?;
    x11_window_pid(window)
}

fn x11_window_pid(window: u32) -> Option<u32> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    let (conn, _) = x11rb::connect(None).ok()?;
    let pid_atom = conn
        .intern_atom(false, b"_NET_WM_PID")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let reply = conn
        .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)
        .ok()
        .and_then(|cookie| cookie.reply().ok())?;
    reply.value32().and_then(|mut values| values.next())
}

fn x11_stacked_windows(include_frame: bool) -> Option<Vec<(u32, Rect)>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;
    let atom = |name: &str| {
        conn.intern_atom(false, name.as_bytes())
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.atom)
    };
    let stacking = atom("_NET_CLIENT_LIST_STACKING")?;
    let wm_type = atom("_NET_WM_WINDOW_TYPE")?;
    let type_normal = atom("_NET_WM_WINDOW_TYPE_NORMAL")?;
    let type_dialog = atom("_NET_WM_WINDOW_TYPE_DIALOG")?;
    let type_utility = atom("_NET_WM_WINDOW_TYPE_UTILITY")?;
    let wm_state = atom("_NET_WM_STATE")?;
    let state_hidden = atom("_NET_WM_STATE_HIDDEN")?;
    let wm_desktop = atom("_NET_WM_DESKTOP")?;
    let current_desktop = atom("_NET_CURRENT_DESKTOP")?;
    let net_frame = atom("_NET_FRAME_EXTENTS")?;
    let gtk_frame = atom("_GTK_FRAME_EXTENTS")?;

    let cardinal = |window: u32, prop: u32| {
        conn.get_property(false, window, prop, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().and_then(|mut values| values.next()))
    };
    let atom_list = |window: u32, prop: u32| -> Vec<u32> {
        conn.get_property(false, window, prop, AtomEnum::ATOM, 0, 32)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().map(|values| values.collect()))
            .unwrap_or_default()
    };
    let extents = |window: u32, prop: u32| -> Option<[i32; 4]> {
        let values: Vec<u32> = conn
            .get_property(false, window, prop, AtomEnum::CARDINAL, 0, 4)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .collect();
        let [left, right, top, bottom] = values.as_slice() else {
            return None;
        };
        Some([*left as i32, *right as i32, *top as i32, *bottom as i32])
    };

    let ids: Vec<u32> = conn
        .get_property(false, root, stacking, AtomEnum::WINDOW, 0, 1024)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .collect();
    let desktop = cardinal(root, current_desktop);
    let capturable_types = [type_normal, type_dialog, type_utility];
    let total = ids.len();
    let mut type_rejected = 0;
    let mut hidden = 0;
    let mut off_desktop = 0;
    let mut unresolved = 0;

    let mut rects = Vec::new();
    for &id in ids.iter().rev() {
        let types = atom_list(id, wm_type);
        if !capturable_window_type(&types, &capturable_types) {
            type_rejected += 1;
            continue;
        }
        if atom_list(id, wm_state).contains(&state_hidden) {
            hidden += 1;
            continue;
        }
        if let (Some(current), Some(window_desktop)) = (desktop, cardinal(id, wm_desktop)) {
            if window_desktop != current && window_desktop != u32::MAX {
                off_desktop += 1;
                continue;
            }
        }
        let Some(geometry) = conn.get_geometry(id).ok().and_then(|c| c.reply().ok()) else {
            unresolved += 1;
            continue;
        };
        let Some(origin) = conn
            .translate_coordinates(id, root, 0, 0)
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            unresolved += 1;
            continue;
        };
        let mut rect = Rect {
            x: origin.dst_x as i32,
            y: origin.dst_y as i32,
            w: geometry.width as i32,
            h: geometry.height as i32,
        };
        if include_frame {
            rect = framed_rect(rect, extents(id, net_frame), extents(id, gtk_frame));
        }
        rects.push((id, rect));
    }
    qol_runtime::probe!(
        "SHOT_SELECT_DISCOVERY",
        "total={total} accepted={} type_rejected={type_rejected} hidden={hidden} off_desktop={off_desktop} unresolved={unresolved}",
        rects.len()
    );
    Some(rects)
}

fn capturable_window_type(types: &[u32], accepted: &[u32]) -> bool {
    types.is_empty()
        || types
            .iter()
            .any(|window_type| accepted.contains(window_type))
}

fn framed_rect(client: Rect, wm_extents: Option<[i32; 4]>, csd_shadow: Option<[i32; 4]>) -> Rect {
    let mut rect = client;
    if let Some([left, right, top, bottom]) = wm_extents {
        rect.x -= left;
        rect.y -= top;
        rect.w += left + right;
        rect.h += top + bottom;
    }
    if let Some([left, right, top, bottom]) = csd_shadow {
        rect.x += left;
        rect.y += top;
        rect.w -= left + right;
        rect.h -= top + bottom;
    }
    rect
}

pub(crate) fn default_window_target(selector_bounds: Bounds<Pixels>) -> Option<Rect> {
    runtime_focused_window_target(selector_bounds)
        .or_else(|| xdotool_active_window_target(selector_bounds))
}

fn runtime_focused_window_target(selector_bounds: Bounds<Pixels>) -> Option<Rect> {
    let window = qol_runtime::PlatformStateClient::from_env()
        .get_state()?
        .focused_window?;
    let rect = Rect {
        x: window.x.round() as i32,
        y: window.y.round() as i32,
        w: window.width.round() as i32,
        h: window.height.round() as i32,
    };
    usable_target("runtime", rect, selector_bounds)
}

fn xdotool_active_window_target(selector_bounds: Bounds<Pixels>) -> Option<Rect> {
    let rect = parse_xdotool_geometry(&run_xdotool(&[
        "getactivewindow",
        "getwindowgeometry",
        "--shell",
    ])?)?;
    usable_target("xdotool", rect, selector_bounds)
}

fn run_xdotool(args: &[&str]) -> Option<String> {
    let program = resolve_command("xdotool")?;
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn parse_xdotool_geometry(output: &str) -> Option<Rect> {
    let x = parse_shell_i32(output, "X")?;
    let y = parse_shell_i32(output, "Y")?;
    let w = parse_shell_i32(output, "WIDTH")?;
    let h = parse_shell_i32(output, "HEIGHT")?;
    positive_rect(Rect { x, y, w, h })
}

fn parse_shell_i32(output: &str, key: &str) -> Option<i32> {
    output.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        (candidate == key).then(|| value.parse().ok()).flatten()
    })
}

fn positive_rect(rect: Rect) -> Option<Rect> {
    (rect.w > 0 && rect.h > 0).then_some(rect)
}

fn usable_target(
    source: &'static str,
    rect: Rect,
    selector_bounds: Bounds<Pixels>,
) -> Option<Rect> {
    let rect = positive_rect(rect)?;
    let usable = rect.w >= MIN_DETECTED_TARGET_PX
        && rect.h >= MIN_DETECTED_TARGET_PX
        && rect_intersects_bounds(rect, selector_bounds);
    trace_target_candidate(source, rect, selector_bounds, usable);
    usable.then_some(rect)
}

fn rect_intersects_bounds(rect: Rect, bounds: Bounds<Pixels>) -> bool {
    let left = rect.x as f32;
    let top = rect.y as f32;
    let right = left + rect.w as f32;
    let bottom = top + rect.h as f32;
    let bounds_left = f32::from(bounds.origin.x);
    let bounds_top = f32::from(bounds.origin.y);
    let bounds_right = bounds_left + f32::from(bounds.size.width);
    let bounds_bottom = bounds_top + f32::from(bounds.size.height);
    left < bounds_right && right > bounds_left && top < bounds_bottom && bottom > bounds_top
}

fn trace_target_candidate(
    source: &'static str,
    rect: Rect,
    selector_bounds: Bounds<Pixels>,
    usable: bool,
) {
    let rect = crate::capture::geometry::rect_label(rect);
    let bounds = rect_from_bounds(selector_bounds);
    let bounds = crate::capture::geometry::rect_label(bounds);
    qol_runtime::probe!(
        "SHOT_SELECT_TARGET",
        "source={source} rect={rect} selector={bounds} usable={usable}"
    );
}

pub fn monitor_capture_rect(monitor: Bounds<Pixels>) -> Option<Rect> {
    positive_rect(rect_from_bounds(monitor))
}

pub(crate) fn rect_from_bounds(bounds: Bounds<Pixels>) -> Rect {
    Rect {
        x: f32::from(bounds.origin.x).round() as i32,
        y: f32::from(bounds.origin.y).round() as i32,
        w: f32::from(bounds.size.width).round() as i32,
        h: f32::from(bounds.size.height).round() as i32,
    }
}

pub fn process_alive(pid: u32) -> bool {
    super::unix::process_alive(pid)
}

#[cfg(test)]
mod tests {
    use super::{
        capturable_window_type, framed_rect, monitor_capture_rect, parse_xdotool_geometry,
        usable_target,
    };
    use crate::Rect;
    use gpui::{point, px, size, Bounds};

    #[test]
    fn window_type_filter_accepts_capture_surfaces() {
        let normal = 1;
        let dialog = 2;
        let utility = 3;
        let dock = 4;
        let menu = 5;
        let tooltip = 6;
        let accepted = [normal, dialog, utility];
        let cases: &[(&[u32], bool)] = &[
            (&[], true),
            (&[normal], true),
            (&[dialog], true),
            (&[utility], true),
            (&[99, dialog], true),
            (&[dock], false),
            (&[menu], false),
            (&[tooltip], false),
            (&[dock, tooltip], false),
        ];

        for (types, expected) in cases {
            assert_eq!(
                capturable_window_type(types, &accepted),
                *expected,
                "types: {types:?}"
            );
        }
    }

    #[test]
    fn monitor_capture_rect_is_the_desktop_pixel_rect_of_the_monitor() {
        let monitor = |x: f32, y: f32, width: f32, height: f32| {
            Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
        };
        let rect = |x: i32, y: i32, w: i32, h: i32| Some(Rect { x, y, w, h });
        let cases = [
            (
                "left monitor, offset down",
                monitor(0.0, 213.0, 1920.0, 1080.0),
                rect(0, 213, 1920, 1080),
            ),
            (
                "right monitor, taller than its neighbour",
                monitor(1920.0, 0.0, 2560.0, 1440.0),
                rect(1920, 0, 2560, 1440),
            ),
            (
                "fractional runtime bounds round to whole pixels",
                monitor(1919.6, 0.4, 2559.7, 1440.2),
                rect(1920, 0, 2560, 1440),
            ),
            (
                "negative origin to the left of the primary",
                monitor(-1920.0, 0.0, 1920.0, 1080.0),
                rect(-1920, 0, 1920, 1080),
            ),
            (
                "zero width has no capture area",
                monitor(0.0, 0.0, 0.0, 1080.0),
                None,
            ),
            (
                "zero height has no capture area",
                monitor(0.0, 0.0, 1920.0, 0.0),
                None,
            ),
        ];
        for (name, monitor, expected) in cases {
            assert_eq!(monitor_capture_rect(monitor), expected, "case: {name}");
        }
    }

    #[test]
    fn parses_xdotool_shell_geometry() {
        assert_eq!(
            parse_xdotool_geometry("WINDOW=123\nX=1930\nY=72\nWIDTH=2560\nHEIGHT=1366\n"),
            Some(Rect {
                x: 1930,
                y: 72,
                w: 2560,
                h: 1366,
            })
        );
    }

    #[test]
    fn rejects_empty_xdotool_geometry() {
        assert_eq!(
            parse_xdotool_geometry("X=0\nY=0\nWIDTH=0\nHEIGHT=1366\n"),
            None
        );
    }

    #[test]
    fn rejects_tiny_detection_targets() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(4480.0), px(1440.0)));
        assert_eq!(
            usable_target(
                "test",
                Rect {
                    x: 3002,
                    y: 370,
                    w: 1,
                    h: 1,
                },
                bounds,
            ),
            None
        );
    }

    #[test]
    fn framed_rect_applies_wm_extents_and_csd_shadows() {
        let client = Rect {
            x: 100,
            y: 128,
            w: 800,
            h: 572,
        };
        let cases = [
            (None, None, client),
            (
                Some([2, 2, 28, 2]),
                None,
                Rect {
                    x: 98,
                    y: 100,
                    w: 804,
                    h: 602,
                },
            ),
            (
                None,
                Some([26, 26, 23, 29]),
                Rect {
                    x: 126,
                    y: 151,
                    w: 748,
                    h: 520,
                },
            ),
        ];
        for (wm, csd, expected) in cases {
            assert_eq!(
                framed_rect(client, wm, csd),
                expected,
                "wm: {wm:?} csd: {csd:?}"
            );
        }
    }
}
