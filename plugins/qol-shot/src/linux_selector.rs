use anyhow::Result;
use gpui::*;
use qol_gpui::monitor::{ActiveMonitor, MonitorTracker};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::Rect;

const SELECTOR_TITLE: &str = "qol-shot-selector";
const SELECTOR_APP_ID: &str = "qol-tray-shot";
const GUIDE_W: f32 = 520.0;
const GUIDE_H: f32 = 78.0;
const GUIDE_TOP: f32 = 48.0;
const GUIDE_MARGIN_X: f32 = 24.0;
const GUIDE_CONTENT_X: f32 = 18.0;
const GUIDE_TITLE_TOP: f32 = 12.0;
const GUIDE_TITLE_H: f32 = 28.0;
const GUIDE_SUBTITLE_TOP: f32 = 46.0;
const GUIDE_SUBTITLE_H: f32 = 20.0;
const LABEL_MIN_W: f32 = 180.0;
const LABEL_MIN_H: f32 = 80.0;

pub fn select_region_blocking() -> Result<Option<Rect>> {
    let (tx, rx) = mpsc::channel();
    Application::new().run(move |cx: &mut App| {
        qol_gpui::platform::set_accessory_policy();
        let monitor = MonitorTracker::start(cx).snapshot_monitor();
        open_region_selector_with_sender(tx, true, monitor, cx);
    });
    Ok(rx.recv().ok().flatten())
}

pub fn open_region_selector(
    cx: &mut App,
    monitor: Option<ActiveMonitor>,
) -> mpsc::Receiver<Option<Rect>> {
    let (tx, rx) = mpsc::channel();
    open_region_selector_with_sender(tx, false, monitor, cx);
    rx
}

fn open_region_selector_with_sender(
    tx: mpsc::Sender<Option<Rect>>,
    quit_on_finish: bool,
    monitor: Option<ActiveMonitor>,
    cx: &mut App,
) {
    let bounds = selector_bounds();
    let active_bounds = monitor.map(|monitor| monitor.bounds());
    let title = selector_title();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::PopUp,
        focus: true,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(SELECTOR_APP_ID.to_string()),
        ..Default::default()
    };
    let window_title = title.clone();
    let opened = cx.open_window(options, move |window, cx| {
        window.set_window_title(&window_title);
        let view =
            cx.new(|cx| RegionSelector::new(tx, quit_on_finish, bounds.origin, active_bounds, cx));
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view
    });
    if opened.is_err() {
        return;
    }
    configure_selector_window(title);
    cx.activate(true);
}

fn selector_bounds() -> Bounds<Pixels> {
    match crate::platform::full_screen_bounds() {
        Ok(bounds) => Bounds::new(
            point(px(bounds.x as f32), px(bounds.y as f32)),
            size(px(bounds.w as f32), px(bounds.h as f32)),
        ),
        Err(_) => Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0))),
    }
}

fn selector_title() -> String {
    format!("{}-{}", SELECTOR_TITLE, std::process::id())
}

fn configure_selector_window(title: String) {
    thread::spawn(move || {
        let started = Instant::now();
        for _ in 0..30 {
            if qol_gpui::popup_window::configure_overlay_window(&title) {
                qol_runtime::probe!(
                    "SHOT_SELECT_OVERLAY",
                    "ms={} result=mapped",
                    started.elapsed().as_millis()
                );
                return;
            }
            thread::sleep(Duration::from_millis(40));
        }
        qol_runtime::probe!(
            "SHOT_SELECT_OVERLAY",
            "ms={} result=timeout",
            started.elapsed().as_millis()
        );
    });
}

struct RegionSelector {
    tx: Option<mpsc::Sender<Option<Rect>>>,
    quit_on_finish: bool,
    window_origin: Point<Pixels>,
    active_bounds: Option<Bounds<Pixels>>,
    drag_start: Option<Point<Pixels>>,
    drag_current: Option<Point<Pixels>>,
    focus_handle: FocusHandle,
}

impl RegionSelector {
    fn new(
        tx: mpsc::Sender<Option<Rect>>,
        quit_on_finish: bool,
        window_origin: Point<Pixels>,
        active_bounds: Option<Bounds<Pixels>>,
        cx: &mut Context<Self>,
    ) -> Self {
        qol_runtime::probe!("SHOT_SELECT_START", "path=gpui");
        Self {
            tx: Some(tx),
            quit_on_finish,
            window_origin,
            active_bounds,
            drag_start: None,
            drag_current: None,
            focus_handle: cx.focus_handle(),
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag_start = Some(event.position);
        self.drag_current = Some(event.position);
        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            return;
        }
        if self.drag_start.is_none() {
            return;
        }
        self.drag_current = Some(event.position);
        cx.notify();
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let rect = self
            .drag_start
            .and_then(|start| selected_rect(self.window_origin, start, event.position));
        self.finish(rect, window, cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" | "esc" => self.finish(None, window, cx),
            _ => {}
        }
    }

    fn finish(&mut self, rect: Option<Rect>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(rect);
        }
        match rect {
            Some(rect) => {
                qol_runtime::probe!("SHOT_SELECT_DONE", "path=gpui raw={}x{}", rect.w, rect.h)
            }
            None => qol_runtime::probe!("SHOT_SELECT_CANCEL", "path=gpui"),
        }
        window.remove_window();
        if self.quit_on_finish {
            cx.quit();
        }
    }

    fn guide_title(&self) -> &'static str {
        if self.drag_start.is_some() {
            return "Release mouse to capture";
        }
        "Drag to select capture area"
    }

    fn selection_bounds(&self) -> Option<Bounds<Pixels>> {
        let start = self.drag_start?;
        let current = self.drag_current?;
        let left = start.x.min(current.x);
        let top = start.y.min(current.y);
        let right = start.x.max(current.x);
        let bottom = start.y.max(current.y);
        Some(Bounds::new(
            point(left, top),
            size(right - left, bottom - top),
        ))
    }

    fn guide_frame(&self, window: &Window) -> (f32, f32, f32) {
        let fallback = window.bounds();
        let bounds = self
            .active_bounds
            .unwrap_or_else(|| Bounds::new(point(px(0.0), px(0.0)), fallback.size));
        let local_x = f32::from(bounds.origin.x) - f32::from(self.window_origin.x);
        let local_y = f32::from(bounds.origin.y) - f32::from(self.window_origin.y);
        let monitor_width = f32::from(bounds.size.width);
        let guide_width = (monitor_width - GUIDE_MARGIN_X * 2.0).max(1.0).min(GUIDE_W);
        let guide_left = local_x + (monitor_width - guide_width) / 2.0;
        (guide_left, local_y + GUIDE_TOP, guide_width)
    }
}

impl Focusable for RegionSelector {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RegionSelector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (guide_left, guide_top, guide_width) = self.guide_frame(window);
        let mut root = div()
            .id("shot-region-selector")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .cursor(CursorStyle::Crosshair)
            .bg(rgba(0x0000006b))
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up));

        root = root.child(
            OverlayText {
                title: self.guide_title(),
                subtitle: Some("Press Esc to cancel"),
                title_size: 22.0,
                subtitle_size: 14.0,
            }
            .panel(guide_left, guide_top, guide_width, GUIDE_H),
        );

        if let Some(bounds) = self.selection_bounds() {
            root = root.child(
                div()
                    .absolute()
                    .left(bounds.origin.x)
                    .top(bounds.origin.y)
                    .w(bounds.size.width)
                    .h(bounds.size.height)
                    .border_2()
                    .border_color(rgb(0xffffff))
                    .bg(rgba(0xff4d4d57))
                    .child(
                        div()
                            .absolute()
                            .left(px(2.0))
                            .top(px(2.0))
                            .w(bounds.size.width - px(4.0))
                            .h(bounds.size.height - px(4.0))
                            .border_2()
                            .border_color(rgb(0xff4d4d)),
                    ),
            );
            if bounds.size.width >= px(LABEL_MIN_W) && bounds.size.height >= px(LABEL_MIN_H) {
                let label_top =
                    f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0 - 13.0;
                root = root.child(
                    OverlayText {
                        title: "Capture area",
                        subtitle: None,
                        title_size: 18.0,
                        subtitle_size: 14.0,
                    }
                    .label(
                        f32::from(bounds.origin.x) + 12.0,
                        label_top,
                        f32::from(bounds.size.width) - 24.0,
                        26.0,
                    ),
                );
            }
        }

        root
    }
}

impl Drop for RegionSelector {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(None);
        }
    }
}

struct OverlayText {
    title: &'static str,
    subtitle: Option<&'static str>,
    title_size: f32,
    subtitle_size: f32,
}

impl OverlayText {
    fn panel(self, left: f32, top: f32, width: f32, height: f32) -> impl IntoElement {
        let content_width = width - GUIDE_CONTENT_X * 2.0;
        let mut panel = div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height))
            .rounded(px(14.0))
            .border_1()
            .border_color(rgba(0xffffffdb))
            .bg(rgba(0x000000c7))
            .relative();

        panel = panel.child(
            div()
                .absolute()
                .left(px(GUIDE_CONTENT_X))
                .top(px(GUIDE_TITLE_TOP))
                .w(px(content_width))
                .h(px(GUIDE_TITLE_H))
                .flex()
                .items_center()
                .justify_center()
                .text_center()
                .text_size(px(self.title_size))
                .line_height(px(GUIDE_TITLE_H))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(0xffffff))
                .child(self.title),
        );

        if let Some(subtitle) = self.subtitle {
            panel = panel.child(
                div()
                    .absolute()
                    .left(px(GUIDE_CONTENT_X))
                    .top(px(GUIDE_SUBTITLE_TOP))
                    .w(px(content_width))
                    .h(px(GUIDE_SUBTITLE_H))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_center()
                    .text_size(px(self.subtitle_size))
                    .line_height(px(GUIDE_SUBTITLE_H))
                    .text_color(rgba(0xffffffc7))
                    .child(subtitle),
            );
        }

        panel
    }

    fn label(self, left: f32, top: f32, width: f32, height: f32) -> impl IntoElement {
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height))
            .flex()
            .items_center()
            .justify_center()
            .text_center()
            .text_size(px(self.title_size))
            .line_height(px(height))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgba(0xfffffff5))
            .child(self.title)
    }
}

fn selected_rect(origin: Point<Pixels>, start: Point<Pixels>, end: Point<Pixels>) -> Option<Rect> {
    let left = start.x.min(end.x).to_f64() + origin.x.to_f64();
    let top = start.y.min(end.y).to_f64() + origin.y.to_f64();
    let right = start.x.max(end.x).to_f64() + origin.x.to_f64();
    let bottom = start.y.max(end.y).to_f64() + origin.y.to_f64();
    let rect = Rect {
        x: left.round() as i32,
        y: top.round() as i32,
        w: (right - left).round() as i32,
        h: (bottom - top).round() as i32,
    };
    if rect.w <= 0 || rect.h <= 0 {
        return None;
    }
    Some(rect)
}
