use gpui::*;
use launcher::open_window_with_focus;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use std::env;
#[cfg(target_os = "linux")]
use std::process::Command;
#[cfg(target_os = "linux")]
use x11rb::connection::Connection;
#[cfg(target_os = "linux")]
use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, ModMask};

actions!(test, [Quit, OpenLauncher]);

const POLL_INTERVAL_MS: u64 = 100;
#[cfg(target_os = "linux")]
const HOTKEY_KEYCODE: u8 = 96;
const LAUNCHER_WIDTH: f32 = 600.0;
const LAUNCHER_HEIGHT: f32 = 42.0;

#[derive(Clone)]
struct ClickInfo {
    global: Point<Pixels>,
    display_id: Option<DisplayId>,
}

#[derive(Clone)]
struct FocusSnapshot {
    window_id: Option<u64>,
    bounds: Bounds<Pixels>,
}

#[derive(Clone, PartialEq)]
struct FocusSignature {
    window_id: Option<u64>,
    bounds: Bounds<Pixels>,
}

#[derive(Clone)]
struct BackendStatus {
    summary: String,
    ok: bool,
}

impl BackendStatus {
    fn ok(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            ok: true,
        }
    }

    fn err(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            ok: false,
        }
    }
}

#[cfg(target_os = "linux")]
fn setup_global_hotkey() -> Result<mpsc::Receiver<()>, BackendStatus> {
    if is_wayland_session() {
        return Err(BackendStatus::err("Wayland (no X11 hotkey grab)"));
    }

    let (conn, screen_num) = x11rb::connect(None)
        .map_err(|e| BackendStatus::err(format!("X11 connect failed: {}", e)))?;

    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    let modifiers = [
        ModMask::from(0u16),
        ModMask::LOCK,
        ModMask::M2,
        ModMask::LOCK | ModMask::M2,
    ];

    for modifier in modifiers {
        conn.grab_key(
            false,
            root,
            modifier,
            HOTKEY_KEYCODE,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )
        .map_err(|e| BackendStatus::err(format!("grab_key failed: {}", e)))?;
    }

    conn.flush()
        .map_err(|e| BackendStatus::err(format!("flush failed: {}", e)))?;

    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        loop {
            match conn.wait_for_event() {
                Ok(event) => {
                    if let x11rb::protocol::Event::KeyPress(ev) = event {
                        if ev.detail == HOTKEY_KEYCODE {
                            if tx.send(()).is_err() {
                                break;
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    Ok(rx)
}

#[cfg(target_os = "macos")]
fn setup_global_hotkey() -> Result<mpsc::Receiver<()>, BackendStatus> {
    use std::os::raw::c_void;

    type CGEventRef = *mut c_void;
    type CFMachPortRef = *mut c_void;
    type CFRunLoopSourceRef = *mut c_void;
    type CFRunLoopRef = *mut c_void;
    type CFStringRef = *const c_void;

    const K_CG_SESSION_EVENT_TAP: u32 = 1;
    const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;
    const K_CG_EVENT_KEY_DOWN: u32 = 10;
    const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
    const F12_KEYCODE: i64 = 111;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventTapCreate(
            tap: u32,
            place: u32,
            options: u32,
            events_of_interest: u64,
            callback: extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef,
            user_info: *mut c_void,
        ) -> CFMachPortRef;
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
        fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFMachPortCreateRunLoopSource(
            allocator: *mut c_void,
            port: CFMachPortRef,
            order: i64,
        ) -> CFRunLoopSourceRef;
        fn CFRunLoopGetCurrent() -> CFRunLoopRef;
        fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
        fn CFRunLoopRun();
        static kCFRunLoopCommonModes: CFStringRef;
    }

    static TX: std::sync::OnceLock<mpsc::Sender<()>> = std::sync::OnceLock::new();

    extern "C" fn callback(
        _proxy: *mut c_void,
        event_type: u32,
        event: CGEventRef,
        _user_info: *mut c_void,
    ) -> CGEventRef {
        if event_type == K_CG_EVENT_KEY_DOWN {
            let keycode = unsafe { CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE) };
            if keycode == F12_KEYCODE {
                if let Some(tx) = TX.get() {
                    let _ = tx.send(());
                }
            }
        }
        event
    }

    let (tx, rx) = mpsc::channel();
    TX.set(tx).map_err(|_| BackendStatus::err("already initialized"))?;

    std::thread::spawn(move || {
        let event_mask: u64 = 1 << K_CG_EVENT_KEY_DOWN;

        let tap = unsafe {
            CGEventTapCreate(
                K_CG_SESSION_EVENT_TAP,
                K_CG_HEAD_INSERT_EVENT_TAP,
                K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
                event_mask,
                callback,
                std::ptr::null_mut(),
            )
        };

        if tap.is_null() {
            eprintln!("CGEventTapCreate failed - grant Accessibility permission in System Settings");
            return;
        }

        unsafe {
            let source = CFMachPortCreateRunLoopSource(std::ptr::null_mut(), tap, 0);
            if source.is_null() {
                eprintln!("CFMachPortCreateRunLoopSource failed");
                return;
            }
            let run_loop = CFRunLoopGetCurrent();
            CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
            CGEventTapEnable(tap, true);
            CFRunLoopRun();
        }
    });

    std::thread::sleep(std::time::Duration::from_millis(200));

    Ok(rx)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn setup_global_hotkey() -> Result<mpsc::Receiver<()>, BackendStatus> {
    Err(BackendStatus::err("unsupported OS"))
}

struct PollResult {
    focus_snapshot: Option<FocusSnapshot>,
    focus_status: BackendStatus,
    click_point: Option<Point<Pixels>>,
    click_status: BackendStatus,
}

#[derive(Clone, Default)]
struct PollState {
    #[cfg(target_os = "linux")]
    button_down: bool,
    #[cfg(target_os = "linux")]
    xinput_device: Option<String>,
}

struct MultiMonitorView {
    focus_handle: FocusHandle,
    last_click: Option<ClickInfo>,
    last_click_at: Option<Instant>,
    focus_snapshot: Option<FocusSnapshot>,
    last_focus_signature: Option<FocusSignature>,
    last_focus_at: Option<Instant>,
    focus_status: BackendStatus,
    click_status: BackendStatus,
    hotkey_status: BackendStatus,
    launcher_status: String,
    launcher_window: Option<WindowHandle<LauncherPopup>>,
    _poll_task: Task<()>,
    _hotkey_task: Option<Task<()>>,
}

impl MultiMonitorView {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();

        let poll_task = cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut state = PollState::default();

            loop {
                if view.upgrade().is_none() {
                    break;
                }

                let (result, next_state) = cx
                    .background_spawn(async move { poll_once(state) })
                    .await;
                state = next_state;

                if let Some(view) = view.upgrade() {
                    let _ = cx.update_entity(&view, |this: &mut MultiMonitorView, cx: &mut Context<MultiMonitorView>| {
                        this.apply_poll(result, cx);
                        cx.notify();
                    });
                } else {
                    break;
                }
            }
        });

        let (hotkey_status, hotkey_task) = match setup_global_hotkey() {
            Ok(rx) => {
                let task = cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
                    loop {
                        if view.upgrade().is_none() {
                            break;
                        }

                        let received = rx.try_recv().ok();

                        if received.is_some() {
                            if let Some(view) = view.upgrade() {
                                let (active_display, click_point, existing_launcher) = cx.update_entity(&view, |this: &mut MultiMonitorView, cx: &mut Context<MultiMonitorView>| {
                                    (this.get_active_display(cx), this.get_recent_click_point(), this.launcher_window)
                                }).ok().unwrap_or((None, None, None));

                                if let Some(handle) = existing_launcher.filter(|h| h.update(cx, |_, _, _| ()).is_ok()) {
                                    let _ = cx.update_window(handle.into(), |_, window, _cx| {
                                        window.remove_window();
                                    });
                                }

                                if let Some(display) = active_display {
                                    let new_handle = cx.update(|cx| {
                                        open_launcher_popup_at(click_point, display, cx)
                                    }).ok().flatten();

                                    let _ = cx.update_entity(&view, |this: &mut MultiMonitorView, cx: &mut Context<MultiMonitorView>| {
                                        this.launcher_window = new_handle;
                                        this.launcher_status = "Popup opened via F12".to_string();
                                        cx.notify();
                                    });
                                }
                            }
                        }

                        cx.background_spawn(async {
                            std::thread::sleep(Duration::from_millis(50));
                        }).await;
                    }
                });
                (BackendStatus::ok("F12 grabbed"), Some(task))
            }
            Err(status) => (status, None),
        };

        Self {
            focus_handle,
            last_click: None,
            last_click_at: None,
            focus_snapshot: None,
            last_focus_signature: None,
            last_focus_at: None,
            focus_status: BackendStatus::err("starting"),
            click_status: BackendStatus::err("starting"),
            hotkey_status,
            launcher_status: "Press F12 anywhere to open launcher".to_string(),
            launcher_window: None,
            _poll_task: poll_task,
            _hotkey_task: hotkey_task,
        }
    }

    fn get_active_display(&self, cx: &Context<Self>) -> Option<Rc<dyn PlatformDisplay>> {
        let displays = cx.displays();
        let display_infos = get_display_infos(&displays);

        let click_display_id = self.last_click.as_ref().and_then(|info| info.display_id);
        let click_is_recent = self
            .last_click_at
            .map(|t| t.elapsed() < Duration::from_secs(5))
            .unwrap_or(false);

        if click_is_recent {
            if let Some(id) = click_display_id {
                return displays.iter().find(|d| d.id() == id).cloned();
            }
        }

        let focus_display_id = self
            .focus_snapshot
            .as_ref()
            .and_then(|snapshot| focused_display_from_infos(&snapshot.bounds, &display_infos))
            .map(|(id, _)| id);

        let active = resolve_active_display(
            focus_display_id,
            self.last_focus_at,
            click_display_id,
            self.last_click_at,
        );

        active.and_then(|(id, _)| displays.iter().find(|d| d.id() == id).cloned())
    }

    fn get_recent_click_point(&self) -> Option<Point<Pixels>> {
        let is_recent = self
            .last_click_at
            .map(|t| t.elapsed() < Duration::from_secs(5))
            .unwrap_or(false);

        if is_recent {
            self.last_click.as_ref().map(|info| info.global)
        } else {
            None
        }
    }

    fn apply_poll(&mut self, poll: PollResult, cx: &mut Context<Self>) {
        self.focus_status = poll.focus_status;
        self.click_status = poll.click_status;

        if let Some(snapshot) = poll.focus_snapshot {
            let signature = FocusSignature {
                window_id: snapshot.window_id,
                bounds: snapshot.bounds,
            };

            if self.last_focus_signature.as_ref() != Some(&signature) {
                self.last_focus_at = Some(Instant::now());
                self.last_focus_signature = Some(signature);
            }

            self.focus_snapshot = Some(snapshot);
        } else {
            self.focus_snapshot = None;
            self.last_focus_signature = None;
            self.last_focus_at = None;
        }

        if let Some(point) = poll.click_point {
            let display_infos = get_display_infos(&cx.displays());
            let display_id = display_for_point_from_infos(&display_infos, point);
            self.last_click = Some(ClickInfo { global: point, display_id });
            self.last_click_at = Some(Instant::now());
        }
    }
}

impl Focusable for MultiMonitorView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MultiMonitorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let displays = cx.displays();
        let display_infos = get_display_infos(&displays);
        let union = display_union_from_infos(&display_infos);
        let window_bounds = window.bounds();
        let window_active = window.is_window_active();

        let focus_display = self
            .focus_snapshot
            .as_ref()
            .and_then(|snapshot| focused_display_from_infos(&snapshot.bounds, &display_infos));
        let focus_display_id = focus_display.map(|(id, _)| id);
        let click_display_id = self.last_click.as_ref().and_then(|info| info.display_id);

        let active_display = resolve_active_display(
            focus_display_id,
            self.last_focus_at,
            click_display_id,
            self.last_click_at,
        );

        let header_height = px(205.);
        let padding = px(16.);
        let viewport = window.viewport_size();
        let map_view_height = std::cmp::max(viewport.height - header_height, px(1.));
        let map_view_width = viewport.width;
        let map_width = std::cmp::max(map_view_width - padding * 2.0, px(1.));
        let map_height = std::cmp::max(map_view_height - padding * 2.0, px(1.));

        let (map_origin, scale) = match union.as_ref() {
            Some(union_bounds) => {
                let scale = if union_bounds.size.width <= px(0.) || union_bounds.size.height <= px(0.) {
                    1.0
                } else {
                    (map_width / union_bounds.size.width).min(map_height / union_bounds.size.height)
                };
                let scaled_width = union_bounds.size.width * scale;
                let scaled_height = union_bounds.size.height * scale;
                let offset_x = (map_width - scaled_width).to_f64() * 0.5;
                let offset_y = (map_height - scaled_height).to_f64() * 0.5;
                (
                    point(
                        padding + px(offset_x as f32),
                        padding + px(offset_y as f32),
                    ),
                    scale,
                )
            }
            None => (point(padding, padding), 1.0),
        };

        let primary_id = cx.primary_display().map(|display| display.id());

        let focus_text = match self.focus_snapshot.as_ref() {
            Some(snapshot) => {
                let window_area = bounds_area(&snapshot.bounds);
                let (display_label, overlap_percent) = focus_display
                    .map(|(id, area)| {
                        let percent = if window_area > 0.0 { area / window_area * 100.0 } else { 0.0 };
                        (format!("Display {}", u32::from(id)), percent)
                    })
                    .unwrap_or_else(|| ("none".to_string(), 0.0));

                format!(
                    "Focused window: {} ({:.1}% overlap) id={} bounds=({}, {}, {}, {})",
                    display_label,
                    overlap_percent,
                    snapshot.window_id.map(|id| id.to_string()).unwrap_or_else(|| "?".to_string()),
                    px_i64(snapshot.bounds.origin.x),
                    px_i64(snapshot.bounds.origin.y),
                    px_i64(snapshot.bounds.size.width),
                    px_i64(snapshot.bounds.size.height),
                )
            }
            None => "Focused window: none".to_string(),
        };

        let click_text = match &self.last_click {
            Some(info) => {
                let display_text = info
                    .display_id
                    .map(|id| format!("Display {}", u32::from(id)))
                    .unwrap_or_else(|| "none".to_string());
                let age = self
                    .last_click_at
                    .map(|t| format!("{:.1}s ago", t.elapsed().as_secs_f32()))
                    .unwrap_or_else(|| "?".to_string());
                format!(
                    "Last cursor press: {} at ({}, {}) ({})",
                    display_text,
                    px_i64(info.global.x),
                    px_i64(info.global.y),
                    age,
                )
            }
            None => "Last cursor press: none".to_string(),
        };

        let active_text = match active_display {
            Some((id, source)) => {
                let age = match source {
                    ActiveSource::Focus => self
                        .last_focus_at
                        .map(|t| format!("{:.1}s ago", t.elapsed().as_secs_f32()))
                        .unwrap_or_else(|| "?".to_string()),
                    ActiveSource::Click => self
                        .last_click_at
                        .map(|t| format!("{:.1}s ago", t.elapsed().as_secs_f32()))
                        .unwrap_or_else(|| "?".to_string()),
                };
                format!(
                    "Active monitor (rule): Display {} ({}, {})",
                    u32::from(id),
                    match source {
                        ActiveSource::Focus => "focused window",
                        ActiveSource::Click => "cursor press",
                    },
                    age,
                )
            }
            None => "Active monitor (rule): none".to_string(),
        };

        let window_text = format!(
            "Test window: x={} y={} w={} h={} ({})",
            px_i64(window_bounds.origin.x),
            px_i64(window_bounds.origin.y),
            px_i64(window_bounds.size.width),
            px_i64(window_bounds.size.height),
            if window_active { "active" } else { "inactive" },
        );

        let focus_backend_text = format!("Focus backend: {}", self.focus_status.summary);
        let click_backend_text = format!("Click backend: {}", self.click_status.summary);
        let hotkey_backend_text = format!("Hotkey backend: {}", self.hotkey_status.summary);
        let launcher_text = self.launcher_status.clone();

        let mut map_children: Vec<AnyElement> = Vec::new();

        if let Some(union_bounds) = union.as_ref() {
            for info in display_infos.iter() {
                let bounds = &info.bounds;
                let scaled = scale_bounds(bounds, union_bounds, map_origin, scale);
                let id = info.id;
                let is_focus = focus_display_id == Some(id);
                let is_click = click_display_id == Some(id);
                let is_active = active_display.map(|(active_id, _)| active_id) == Some(id);

                let border_color = if is_focus && is_click {
                    rgb(0xf9e2af)
                } else if is_focus {
                    rgb(0xa6e3a1)
                } else if is_click {
                    rgb(0x89b4fa)
                } else {
                    rgb(0x45475a)
                };

                let bg_color = if is_active { rgb(0x313244) } else { rgb(0x1e1e2e) };
                let label = format!(
                    "Display {}{}",
                    u32::from(id),
                    if primary_id == Some(id) { " (primary)" } else { "" }
                );
                let size_label = format!(
                    "{} x {}",
                    px_i64(bounds.size.width),
                    px_i64(bounds.size.height)
                );
                let origin_label = format!(
                    "x={} y={}",
                    px_i64(bounds.origin.x),
                    px_i64(bounds.origin.y)
                );

                map_children.push(
                    div()
                        .absolute()
                        .left(scaled.origin.x)
                        .top(scaled.origin.y)
                        .w(scaled.size.width)
                        .h(scaled.size.height)
                        .bg(bg_color)
                        .border_1()
                        .border_color(border_color)
                        .rounded_md()
                        .child(
                            div()
                                .p_2()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .text_color(rgb(0xcdd6f4))
                                .text_size(px(12.))
                                .child(label)
                                .child(size_label)
                                .child(origin_label),
                        )
                        .into_any_element(),
                );
            }

            if let Some(snapshot) = self.focus_snapshot.as_ref() {
                let focused_scaled = scale_bounds(&snapshot.bounds, union_bounds, map_origin, scale);
                map_children.push(
                    div()
                        .absolute()
                        .left(focused_scaled.origin.x)
                        .top(focused_scaled.origin.y)
                        .w(focused_scaled.size.width)
                        .h(focused_scaled.size.height)
                        .bg(Rgba { r: 0.6, g: 1.0, b: 0.6, a: 0.08 })
                        .border_1()
                        .border_color(rgb(0xa6e3a1))
                        .child(
                            div()
                                .p_1()
                                .text_color(rgb(0xa6e3a1))
                                .text_size(px(11.))
                                .child("Focused window"),
                        )
                        .into_any_element(),
                );
            }

            let test_scaled = scale_bounds(&window_bounds, union_bounds, map_origin, scale);
            map_children.push(
                div()
                    .absolute()
                    .left(test_scaled.origin.x)
                    .top(test_scaled.origin.y)
                    .w(test_scaled.size.width)
                    .h(test_scaled.size.height)
                    .bg(Rgba { r: 0.6, g: 0.6, b: 0.7, a: 0.06 })
                    .border_1()
                    .border_color(rgb(0x9399b2))
                    .child(
                        div()
                            .p_1()
                            .text_color(rgb(0x9399b2))
                            .text_size(px(11.))
                            .child("Test window"),
                    )
                    .into_any_element(),
            );

            if let Some(info) = &self.last_click {
                let click_point = map_point(info.global, union_bounds, map_origin, scale);
                map_children.push(
                    div()
                        .absolute()
                        .left(click_point.x - px(4.))
                        .top(click_point.y - px(4.))
                        .w(px(8.))
                        .h(px(8.))
                        .rounded_full()
                        .bg(rgb(0x89b4fa))
                        .into_any_element(),
                );
            }
        }

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x11111b))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => cx.quit(),
                    "f12" => {
                        if let Some(handle) = this.launcher_window.filter(|h| h.update(cx, |_, _, _| ()).is_ok()) {
                            let _ = handle.update(cx, |_popup, window, _cx| {
                                window.remove_window();
                            });
                        }

                        if let Some(display) = this.get_active_display(cx) {
                            let click_point = this.get_recent_click_point();
                            this.launcher_window = open_launcher_popup_at(click_point, display, cx);
                            this.launcher_status = "Popup opened via F12 (local)".to_string();
                        }
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .child(
                div()
                    .h(header_height)
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_4()
                    .py_3()
                    .bg(rgb(0x181825))
                    .border_b_1()
                    .border_color(rgb(0x45475a))
                    .child(
                        div()
                            .text_color(rgb(0xcdd6f4))
                            .text_size(px(18.))
                            .child("Multi-monitor active test (global input)"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xa6adc8))
                            .text_size(px(12.))
                            .child("Interact with other apps or desktops; active monitor updates from focus or cursor press."),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xa6adc8))
                            .text_size(px(12.))
                            .child("Rule: last event wins (focus overlap vs cursor press)."),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xa6adc8))
                            .text_size(px(12.))
                            .child("Keys: F12=open popup (global), Esc=quit"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xa6adc8))
                            .text_size(px(12.))
                            .child(window_text),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xa6adc8))
                            .text_size(px(12.))
                            .child(focus_text),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xa6adc8))
                            .text_size(px(12.))
                            .child(click_text),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xcdd6f4))
                            .text_size(px(12.))
                            .child(active_text),
                    )
                    .child(
                        div()
                            .text_color(if self.focus_status.ok { rgb(0xa6e3a1) } else { rgb(0xf38ba8) })
                            .text_size(px(12.))
                            .child(focus_backend_text),
                    )
                    .child(
                        div()
                            .text_color(if self.click_status.ok { rgb(0xa6e3a1) } else { rgb(0xf38ba8) })
                            .text_size(px(12.))
                            .child(click_backend_text),
                    )
                    .child(
                        div()
                            .text_color(if self.hotkey_status.ok { rgb(0xa6e3a1) } else { rgb(0xf38ba8) })
                            .text_size(px(12.))
                            .child(hotkey_backend_text),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xa6adc8))
                            .text_size(px(12.))
                            .child(launcher_text),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .text_size(px(11.))
                            .text_color(rgb(0x9399b2))
                            .child("Legend:")
                            .child(
                                div()
                                    .text_color(rgb(0xa6e3a1))
                                    .child("focused"),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0x89b4fa))
                                    .child("cursor press"),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0xf9e2af))
                                    .child("both"),
                            ),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .w_full()
                    .bg(rgb(0x0f0f17))
                    .children(map_children),
            )
    }
}

#[derive(Clone, Copy)]
enum ActiveSource {
    Focus,
    Click,
}

fn resolve_active_display(
    focus_id: Option<DisplayId>,
    focus_time: Option<Instant>,
    click_id: Option<DisplayId>,
    click_time: Option<Instant>,
) -> Option<(DisplayId, ActiveSource)> {
    let focus = focus_id.zip(focus_time);
    let click = click_id.zip(click_time);
    match (focus, click) {
        (Some((fid, ft)), Some((cid, ct))) => {
            if ft >= ct {
                Some((fid, ActiveSource::Focus))
            } else {
                Some((cid, ActiveSource::Click))
            }
        }
        (Some((fid, _)), None) => Some((fid, ActiveSource::Focus)),
        (None, Some((cid, _))) => Some((cid, ActiveSource::Click)),
        _ => None,
    }
}

#[derive(Clone)]
struct DisplayInfo {
    id: DisplayId,
    bounds: Bounds<Pixels>,
}

fn get_display_infos(displays: &[Rc<dyn PlatformDisplay>]) -> Vec<DisplayInfo> {
    #[cfg(target_os = "macos")]
    {
        let cg_bounds = get_macos_display_bounds();
        displays
            .iter()
            .enumerate()
            .map(|(i, display)| {
                let bounds = cg_bounds.get(i).cloned().unwrap_or_else(|| display.bounds());
                DisplayInfo {
                    id: display.id(),
                    bounds,
                }
            })
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        displays
            .iter()
            .map(|display| DisplayInfo {
                id: display.id(),
                bounds: display.bounds(),
            })
            .collect()
    }
}

fn display_union_from_infos(infos: &[DisplayInfo]) -> Option<Bounds<Pixels>> {
    let mut iter = infos.iter();
    let first = iter.next()?.bounds;
    Some(iter.fold(first, |acc, info| acc.union(&info.bounds)))
}

fn display_for_point_from_infos(
    infos: &[DisplayInfo],
    point: Point<Pixels>,
) -> Option<DisplayId> {
    infos
        .iter()
        .find(|info| info.bounds.contains(&point))
        .map(|info| info.id)
}

fn focused_display_from_infos(
    window_bounds: &Bounds<Pixels>,
    infos: &[DisplayInfo],
) -> Option<(DisplayId, f64)> {
    let mut best: Option<(DisplayId, f64)> = None;
    for info in infos.iter() {
        let area = intersection_area(window_bounds, &info.bounds);
        if area <= 0.0 {
            continue;
        }
        match best {
            Some((_, best_area)) if best_area >= area => {}
            _ => best = Some((info.id, area)),
        }
    }
    best
}

fn intersection_area(a: &Bounds<Pixels>, b: &Bounds<Pixels>) -> f64 {
    bounds_area(&a.intersect(b))
}

fn bounds_area(bounds: &Bounds<Pixels>) -> f64 {
    if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
        return 0.0;
    }
    bounds.size.width.to_f64() * bounds.size.height.to_f64()
}

fn scale_bounds(
    bounds: &Bounds<Pixels>,
    union: &Bounds<Pixels>,
    map_origin: Point<Pixels>,
    scale: f32,
) -> Bounds<Pixels> {
    let rel_x = bounds.origin.x - union.origin.x;
    let rel_y = bounds.origin.y - union.origin.y;
    Bounds::new(
        point(map_origin.x + rel_x * scale, map_origin.y + rel_y * scale),
        size(bounds.size.width * scale, bounds.size.height * scale),
    )
}

fn map_point(
    global: Point<Pixels>,
    union: &Bounds<Pixels>,
    map_origin: Point<Pixels>,
    scale: f32,
) -> Point<Pixels> {
    let rel_x = global.x - union.origin.x;
    let rel_y = global.y - union.origin.y;
    point(map_origin.x + rel_x * scale, map_origin.y + rel_y * scale)
}

fn px_i64(value: Pixels) -> i64 {
    value.to_f64().round() as i64
}

fn poll_once(mut state: PollState) -> (PollResult, PollState) {
    let (focus_snapshot, focus_status) = poll_focus();
    let (click_point, click_status) = poll_click(&mut state);

    std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));

    (
        PollResult {
            focus_snapshot,
            focus_status,
            click_point,
            click_status,
        },
        state,
    )
}

#[cfg(target_os = "linux")]
fn poll_focus() -> (Option<FocusSnapshot>, BackendStatus) {
    if is_wayland_session() {
        return (None, BackendStatus::err("Wayland session (focus unavailable)"));
    }
    if !command_exists("xdotool") {
        return (None, BackendStatus::err("xdotool not found"));
    }

    let output = Command::new("xdotool")
        .args(["getactivewindow", "getwindowgeometry", "--shell"])
        .output();

    let Ok(out) = output else {
        return (None, BackendStatus::err("xdotool failed"));
    };

    if !out.status.success() {
        return (None, BackendStatus::err("xdotool failed"));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let window_id = parse_shell_u64(&stdout, "WINDOW");
    let x = parse_shell_i32(&stdout, "X");
    let y = parse_shell_i32(&stdout, "Y");
    let w = parse_shell_i32(&stdout, "WIDTH");
    let h = parse_shell_i32(&stdout, "HEIGHT");

    match (x, y, w, h) {
        (Some(x), Some(y), Some(w), Some(h)) if w > 0 && h > 0 => (
            Some(FocusSnapshot {
                window_id,
                bounds: Bounds::new(
                    point(px(x as f32), px(y as f32)),
                    size(px(w as f32), px(h as f32)),
                ),
            }),
            BackendStatus::ok("xdotool"),
        ),
        _ => (None, BackendStatus::err("xdotool parse error")),
    }
}

#[cfg(target_os = "macos")]
fn poll_focus() -> (Option<FocusSnapshot>, BackendStatus) {
    use std::process::Command;

    let script = r#"
        tell application "System Events"
            set frontApp to first application process whose frontmost is true
            set frontAppName to name of frontApp
            try
                set frontWindow to first window of frontApp
                set {x, y} to position of frontWindow
                set {w, h} to size of frontWindow
                return frontAppName & "|" & x & "," & y & "," & w & "," & h
            on error
                return "no-window"
            end try
        end tell
    "#;

    let output = Command::new("osascript")
        .args(["-e", script])
        .output();

    let Ok(out) = output else {
        return (None, BackendStatus::err("osascript failed"));
    };

    if !out.status.success() {
        return (None, BackendStatus::err("osascript failed"));
    }

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if stdout == "no-window" || stdout.is_empty() {
        return (None, BackendStatus::ok("osascript (no window)"));
    }

    let pipe_parts: Vec<&str> = stdout.splitn(2, '|').collect();
    let (app_name, coords_str) = if pipe_parts.len() == 2 {
        (pipe_parts[0], pipe_parts[1])
    } else {
        ("?", stdout.as_str())
    };

    let parts: Vec<&str> = coords_str.split(',').collect();
    if parts.len() != 4 {
        return (None, BackendStatus::err("osascript parse error"));
    }

    let x = parts[0].parse::<i32>().ok();
    let y = parts[1].parse::<i32>().ok();
    let w = parts[2].parse::<i32>().ok();
    let h = parts[3].parse::<i32>().ok();

    match (x, y, w, h) {
        (Some(x), Some(y), Some(w), Some(h)) if w > 0 && h > 0 => {
            println!("Focus: {} at ({}, {}, {}, {})", app_name, x, y, w, h);
            (
                Some(FocusSnapshot {
                    window_id: None,
                    bounds: Bounds::new(
                        point(px(x as f32), px(y as f32)),
                        size(px(w as f32), px(h as f32)),
                    ),
                }),
                BackendStatus::ok("osascript"),
            )
        }
        _ => (None, BackendStatus::err("osascript parse error")),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn poll_focus() -> (Option<FocusSnapshot>, BackendStatus) {
    (None, BackendStatus::err("unsupported OS"))
}

#[cfg(target_os = "linux")]
fn poll_click(state: &mut PollState) -> (Option<Point<Pixels>>, BackendStatus) {
    if is_wayland_session() {
        return (None, BackendStatus::err("Wayland session (clicks unavailable)"));
    }
    if !command_exists("xinput") || !command_exists("xdotool") {
        return (None, BackendStatus::err("xinput/xdotool not found"));
    }

    let device_id = match state.xinput_device.clone() {
        Some(id) => id,
        None => match detect_xinput_device_id() {
            Some(id) => {
                state.xinput_device = Some(id.clone());
                id
            }
            None => return (None, BackendStatus::err("xinput device not found")),
        },
    };

    let output = Command::new("xinput")
        .args(["--query-state", &device_id])
        .output();

    let Ok(out) = output else {
        state.xinput_device = None;
        return (None, BackendStatus::err("xinput query failed"));
    };

    if !out.status.success() {
        state.xinput_device = None;
        return (None, BackendStatus::err("xinput query failed"));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let button_down = parse_button_down(&stdout);

    let mut click_point = None;
    if button_down && !state.button_down {
        click_point = query_mouse_location();
    }

    state.button_down = button_down;

    (click_point, BackendStatus::ok("xinput + xdotool"))
}

#[cfg(target_os = "macos")]
fn poll_click(_state: &mut PollState) -> (Option<Point<Pixels>>, BackendStatus) {
    (None, BackendStatus::err("macOS (click tracking not implemented)"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn poll_click(_state: &mut PollState) -> (Option<Point<Pixels>>, BackendStatus) {
    (None, BackendStatus::err("unsupported OS"))
}

#[cfg(target_os = "linux")]
fn detect_xinput_device_id() -> Option<String> {
    let output = Command::new("xinput").args(["list", "--short"]).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        let dominated = line.to_lowercase();
        if line.contains("slave  pointer")
            && !line.contains("XTEST")
            && (dominated.contains("mouse") || dominated.contains("logitech") || dominated.contains("razer"))
        {
            if let Some(id) = parse_xinput_id(line) {
                return Some(id);
            }
        }
    }

    for line in stdout.lines() {
        if line.contains("slave  pointer") && !line.contains("XTEST") && !line.contains("Consumer") && !line.contains("Keyboard") {
            if let Some(id) = parse_xinput_id(line) {
                return Some(id);
            }
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn parse_xinput_id(line: &str) -> Option<String> {
    let id_part = line.split("id=").nth(1)?;
    let id = id_part.split_whitespace().next()?;
    Some(id.to_string())
}

#[cfg(target_os = "linux")]
fn parse_button_down(output: &str) -> bool {
    output
        .lines()
        .any(|line| line.contains("button[") && line.contains("]=down"))
}

#[cfg(target_os = "linux")]
fn query_mouse_location() -> Option<Point<Pixels>> {
    let output = Command::new("xdotool")
        .args(["getmouselocation", "--shell"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let x = parse_shell_i32(&stdout, "X")?;
    let y = parse_shell_i32(&stdout, "Y")?;
    Some(point(px(x as f32), px(y as f32)))
}

#[cfg(target_os = "linux")]
fn parse_shell_i32(output: &str, key: &str) -> Option<i32> {
    for line in output.lines() {
        if let Some(val) = line.strip_prefix(&format!("{}=", key)) {
            return val.trim().parse::<i32>().ok();
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn parse_shell_u64(output: &str, key: &str) -> Option<u64> {
    for line in output.lines() {
        if let Some(val) = line.strip_prefix(&format!("{}=", key)) {
            return val.trim().parse::<u64>().ok();
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn is_wayland_session() -> bool {
    env::var("XDG_SESSION_TYPE").map(|val| val == "wayland").unwrap_or(false)
        || env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(target_os = "linux")]
fn command_exists(cmd: &str) -> bool {
    env::var_os("PATH")
        .and_then(|paths| {
            env::split_paths(&paths).find(|path| path.join(cmd).is_file())
        })
        .is_some()
}

struct LauncherPopup {
    focus_handle: FocusHandle,
    blur_sub: Option<Subscription>,
}

impl LauncherPopup {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            blur_sub: None,
        }
    }
}

impl Focusable for LauncherPopup {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LauncherPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.blur_sub.is_none() {
            self.blur_sub = Some(cx.on_blur(&self.focus_handle, window, |_this, window, _cx| {
                window.remove_window();
            }));
        }

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(0x1e1e2e))
            .border_1()
            .border_color(rgb(0x45475a))
            .rounded_lg()
            .flex()
            .items_center()
            .px_4()
            .on_key_down(cx.listener(|_this, _event: &KeyDownEvent, window, _cx| {
                window.remove_window();
            }))
            .child(
                div()
                    .text_color(rgb(0xcdd6f4))
                    .text_size(px(14.))
                    .child("Launcher popup (click outside or press any key to close)"),
            )
    }
}

fn open_launcher_popup_at(click_point: Option<Point<Pixels>>, display: Rc<dyn PlatformDisplay>, cx: &mut App) -> Option<WindowHandle<LauncherPopup>> {
    let display_infos = get_display_infos(&cx.displays());
    let display_bounds = display_infos
        .iter()
        .find(|info| info.id == display.id())
        .map(|info| info.bounds)
        .unwrap_or_else(|| display.bounds());

    let center_x = if let Some(click) = click_point {
        click.x - px(LAUNCHER_WIDTH / 2.0)
    } else {
        display_bounds.origin.x + (display_bounds.size.width - px(LAUNCHER_WIDTH)) / 2.0
    };
    let center_y = display_bounds.origin.y + (display_bounds.size.height - px(LAUNCHER_HEIGHT)) / 3.0;

    let bounds = Bounds::new(
        point(center_x, center_y),
        size(px(LAUNCHER_WIDTH), px(LAUNCHER_HEIGHT)),
    );

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::PopUp,
        focus: true,
        ..Default::default()
    };

    cx.open_window(options, |_window, cx| cx.new(|cx| LauncherPopup::new(cx))).ok()
}

#[cfg(target_os = "macos")]
fn get_macos_display_bounds() -> Vec<Bounds<Pixels>> {

    #[repr(C)]
    #[derive(Debug, Copy, Clone)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[repr(C)]
    #[derive(Debug, Copy, Clone)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Debug, Copy, Clone)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    type CGDirectDisplayID = u32;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGGetActiveDisplayList(
            max_displays: u32,
            displays: *mut CGDirectDisplayID,
            display_count: *mut u32,
        ) -> i32;
        fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    }

    let mut display_ids: [CGDirectDisplayID; 16] = [0; 16];
    let mut display_count: u32 = 0;

    unsafe {
        let result = CGGetActiveDisplayList(16, display_ids.as_mut_ptr(), &mut display_count);
        if result != 0 {
            return Vec::new();
        }
    }

    let mut bounds_list = Vec::new();
    for i in 0..display_count as usize {
        let rect = unsafe { CGDisplayBounds(display_ids[i]) };
        bounds_list.push(Bounds::new(
            point(px(rect.origin.x as f32), px(rect.origin.y as f32)),
            size(px(rect.size.width as f32), px(rect.size.height as f32)),
        ));
    }

    bounds_list
}

#[cfg(not(target_os = "macos"))]
fn get_macos_display_bounds() -> Vec<Bounds<Pixels>> {
    Vec::new()
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let displays = cx.displays();
        println!("gpui detected {} display(s):", displays.len());
        for (i, display) in displays.iter().enumerate() {
            let bounds = display.bounds();
            println!(
                "  Display {} (gpui): id={} bounds=({}, {}, {}, {})",
                i,
                u32::from(display.id()),
                bounds.origin.x.to_f64() as i32,
                bounds.origin.y.to_f64() as i32,
                bounds.size.width.to_f64() as i32,
                bounds.size.height.to_f64() as i32,
            );
        }

        #[cfg(target_os = "macos")]
        {
            let cg_bounds = get_macos_display_bounds();
            println!("CoreGraphics detected {} display(s):", cg_bounds.len());
            for (i, bounds) in cg_bounds.iter().enumerate() {
                println!(
                    "  Display {} (CG): bounds=({}, {}, {}, {})",
                    i,
                    bounds.origin.x.to_f64() as i32,
                    bounds.origin.y.to_f64() as i32,
                    bounds.size.width.to_f64() as i32,
                    bounds.size.height.to_f64() as i32,
                );
            }
        }

        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(980.), px(720.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(640.), px(460.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Multi-monitor test".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        open_window_with_focus(cx, options, |window, cx| MultiMonitorView::new(window, cx)).unwrap();

        cx.activate(true);
    });
}
