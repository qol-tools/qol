use anyhow::Result;
use gpui::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crate::{Monitor, Rect};

const SELECTOR_TITLE: &str = "qol-shot-selector";
const SELECTOR_TITLE_PREFIX: &str = "qol-shot-selector-";
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
const BACKDROP_COLOR: u32 = 0x2f80ed24;
const HIDE_BARRIER_CLEAR_SAMPLES: usize = 3;
const HIDE_BARRIER_MAX_MS: u64 = 750;
static SELECTOR_SEQ: AtomicU64 = AtomicU64::new(0);

pub type RectMapper = Rc<dyn Fn(Rect) -> Option<Rect>>;

pub trait GlobalPointer {
    fn position(&self) -> Option<Point<Pixels>>;
    fn primary_button_down(&self) -> bool;
}

pub type GlobalPointerSource = Rc<dyn GlobalPointer>;

pub struct SelectorWindowOptions {
    pub display_id: Option<DisplayId>,
    pub kind: WindowKind,
    pub decorations: WindowDecorations,
    pub focus: bool,
}

pub struct SelectorWindow {
    title: String,
    bounds: Bounds<Pixels>,
    active_bounds: Option<Bounds<Pixels>>,
    display_id: Option<DisplayId>,
    kind: WindowKind,
    decorations: WindowDecorations,
    focus: bool,
    map_rect: RectMapper,
    global_pointer: Option<GlobalPointerSource>,
}

impl SelectorWindow {
    pub fn new(
        bounds: Bounds<Pixels>,
        active_bounds: Option<Bounds<Pixels>>,
        options: SelectorWindowOptions,
        map_rect: RectMapper,
        global_pointer: Option<GlobalPointerSource>,
    ) -> Self {
        Self {
            title: selector_title(),
            bounds,
            active_bounds,
            display_id: options.display_id,
            kind: options.kind,
            decorations: options.decorations,
            focus: options.focus,
            map_rect,
            global_pointer,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    fn options(&self) -> WindowOptions {
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(self.bounds)),
            titlebar: None,
            window_decorations: Some(self.decorations),
            kind: self.kind,
            focus: self.focus,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: self.display_id,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some(SELECTOR_APP_ID.to_string()),
            ..Default::default()
        }
    }
}

pub fn select_region_blocking_with<F>(open_selector: F) -> Result<Option<Rect>>
where
    F: FnOnce(mpsc::Sender<Option<Rect>>, &mut App) + 'static,
{
    let (tx, rx) = mpsc::channel();
    Application::new().run(move |cx: &mut App| {
        qol_gpui::platform::set_accessory_policy();
        open_selector(tx, cx);
    });
    Ok(rx.recv().ok().flatten())
}

pub fn open_all(
    tx: mpsc::Sender<Option<Rect>>,
    quit_on_finish: bool,
    selectors: Vec<SelectorWindow>,
    cx: &mut App,
) -> bool {
    let selector_count = selectors.len();
    let active_bounds = selectors.iter().find_map(|selector| selector.active_bounds);
    let monitor_bounds = selectors
        .iter()
        .map(|selector| selector.bounds)
        .collect::<Vec<_>>();
    let titles = selectors
        .iter()
        .map(|selector| selector.title.clone())
        .collect::<Vec<_>>();
    let state = Rc::new(RefCell::new(SelectionState::new(
        tx,
        active_bounds,
        monitor_bounds,
        titles,
    )));
    let mut handles = Vec::new();
    for selector in selectors {
        let Some(handle) = open_window(selector, state.clone(), quit_on_finish, cx) else {
            continue;
        };
        handles.push(handle);
    }
    if handles.is_empty() {
        qol_runtime::probe!("SHOT_SELECT_OPEN", "selectors={selector_count} result=none");
        return false;
    }
    qol_runtime::probe!(
        "SHOT_SELECT_OPEN",
        "selectors={selector_count} windows={} result=ok",
        handles.len()
    );
    state.borrow_mut().handles = handles.clone();
    for handle in handles.iter().cloned() {
        let _ = handle.update(cx, |view, _window, _cx| view.handle = Some(handle));
    }
    if let Some(handle) = handles.first().cloned() {
        let _ = handle.update(cx, |view, _window, cx| view.start_active_monitor_poll(cx));
    }
    true
}

fn open_window(
    selector: SelectorWindow,
    state: Rc<RefCell<SelectionState>>,
    quit_on_finish: bool,
    cx: &mut App,
) -> Option<WindowHandle<RegionSelector>> {
    let options = selector.options();
    let focus = selector.focus;
    let map_rect = selector.map_rect;
    let global_pointer = selector.global_pointer;
    let window_title = selector.title;
    cx.open_window(options, move |window, cx| {
        window.set_window_title(&window_title);
        qol_runtime::probe!("SHOT_SELECT_WINDOW", "title={window_title} state=open");
        let view = cx.new(|cx| {
            RegionSelector::new(
                state,
                window_title.clone(),
                quit_on_finish,
                map_rect,
                global_pointer,
                cx,
            )
        });
        if focus {
            window.focus(&view.focus_handle(cx));
            window.activate_window();
        }
        view
    })
    .ok()
}

pub fn bounds_from_monitor(bounds: Monitor) -> Bounds<Pixels> {
    Bounds::new(
        point(px(bounds.x as f32), px(bounds.y as f32)),
        size(px(bounds.w as f32), px(bounds.h as f32)),
    )
}

pub fn fallback_bounds() -> Bounds<Pixels> {
    Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0)))
}

#[cfg(target_os = "linux")]
pub fn identity_rect_mapper() -> RectMapper {
    Rc::new(Some)
}

fn selector_title() -> String {
    let seq = SELECTOR_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{seq}", SELECTOR_TITLE, std::process::id())
}

struct SelectionState {
    tx: Option<mpsc::Sender<Option<Rect>>>,
    active_bounds: Option<Bounds<Pixels>>,
    monitor_bounds: Vec<Bounds<Pixels>>,
    titles: Vec<String>,
    drag_start: Option<Point<Pixels>>,
    drag_current: Option<Point<Pixels>>,
    handles: Vec<WindowHandle<RegionSelector>>,
    polling: bool,
    active_monitor_polling: bool,
}

impl SelectionState {
    fn new(
        tx: mpsc::Sender<Option<Rect>>,
        active_bounds: Option<Bounds<Pixels>>,
        monitor_bounds: Vec<Bounds<Pixels>>,
        titles: Vec<String>,
    ) -> Self {
        Self {
            tx: Some(tx),
            active_bounds,
            monitor_bounds,
            titles,
            drag_start: None,
            drag_current: None,
            handles: Vec::new(),
            polling: false,
            active_monitor_polling: false,
        }
    }

    fn set_active_bounds(&mut self, bounds: Bounds<Pixels>) -> bool {
        if self.active_bounds == Some(bounds) {
            return false;
        }
        self.active_bounds = Some(bounds);
        true
    }

    fn set_active_bounds_for_point(&mut self, point: Point<Pixels>) -> bool {
        let Some(bounds) = monitor_bounds_for_point(&self.monitor_bounds, point) else {
            return false;
        };
        self.set_active_bounds(bounds)
    }
}

enum GlobalDragResult {
    Continue,
    Stop,
    Finish {
        handles: Vec<WindowHandle<RegionSelector>>,
        completion: SelectionCompletion,
    },
}

struct SelectionCompletion {
    tx: mpsc::Sender<Option<Rect>>,
    rect: Option<Rect>,
    quit_on_finish: bool,
}

impl SelectionCompletion {
    fn finish(self, cx: &mut App) {
        let quit_on_finish = self.quit_on_finish;
        let sent = self.tx.send(self.rect).is_ok();
        qol_runtime::probe!(
            "SHOT_SELECT_COMPLETE",
            "sent={sent} quit_on_finish={quit_on_finish}"
        );
        if quit_on_finish {
            cx.quit();
        }
    }
}

struct RegionSelector {
    state: Rc<RefCell<SelectionState>>,
    handle: Option<WindowHandle<RegionSelector>>,
    title: String,
    quit_on_finish: bool,
    map_rect: RectMapper,
    global_pointer: Option<GlobalPointerSource>,
    focus_handle: FocusHandle,
}

impl RegionSelector {
    fn new(
        state: Rc<RefCell<SelectionState>>,
        title: String,
        quit_on_finish: bool,
        map_rect: RectMapper,
        global_pointer: Option<GlobalPointerSource>,
        cx: &mut Context<Self>,
    ) -> Self {
        qol_runtime::probe!("SHOT_SELECT_START", "path=gpui");
        Self {
            state,
            handle: None,
            title,
            quit_on_finish,
            map_rect,
            global_pointer,
            focus_handle: cx.focus_handle(),
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = self.global_point(event.position, window);
        {
            let mut state = self.state.borrow_mut();
            state.drag_start = Some(position);
            state.drag_current = Some(position);
            state.set_active_bounds_for_point(position);
        }
        self.notify_all(cx);
        self.start_global_drag(cx);
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = self.global_point(event.position, window);
        let active_changed = self
            .state
            .borrow_mut()
            .set_active_bounds_for_point(position);
        if !event.dragging() {
            if active_changed {
                self.notify_all(cx);
            }
            return;
        }
        if self.state.borrow().drag_start.is_none() {
            return;
        }
        self.state.borrow_mut().drag_current = Some(position);
        self.notify_all(cx);
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let end = self.global_point(event.position, window);
        {
            let mut state = self.state.borrow_mut();
            state.drag_current = Some(end);
            state.set_active_bounds_for_point(end);
        }
        let raw = self.current_raw_rect(Some(end));
        let rect = raw.and_then(|rect| (self.map_rect)(rect));
        trace_selection_release("event", raw, rect);
        self.finish(rect, window, cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" | "esc" => self.finish(None, window, cx),
            _ => {}
        }
    }

    fn finish(&mut self, rect: Option<Rect>, window: &mut Window, cx: &mut Context<Self>) {
        let completion = self.take_completion(rect);
        self.hide_window_before_remove(window);
        self.hide_all_before_remove();
        self.remove_peer_windows(cx);
        window.remove_window();
        if let Some(completion) = completion {
            schedule_completion(completion, cx);
        }
    }

    fn start_global_drag(&self, cx: &mut Context<Self>) {
        let Some(pointer) = self.global_pointer.clone() else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            if state.polling {
                return;
            }
            state.polling = true;
        }
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let position = pointer.position();
                    let pressed = pointer.primary_button_down();
                    let result = this
                        .update(&mut cx, |view, cx| {
                            view.apply_global_pointer_sample(position, pressed, cx)
                        })
                        .unwrap_or(GlobalDragResult::Stop);
                    match result {
                        GlobalDragResult::Continue => {}
                        GlobalDragResult::Stop => break,
                        GlobalDragResult::Finish {
                            handles,
                            completion,
                        } => {
                            for handle in handles {
                                let _ = handle.update(&mut cx, |view, window, _cx| {
                                    view.hide_window_before_remove(window);
                                    window.remove_window();
                                });
                            }
                            wait_for_selector_hide_barrier(&mut cx).await;
                            let _ = cx.update(move |cx| {
                                cx.defer(move |cx| completion.finish(cx));
                            });
                            break;
                        }
                    }
                }
            }
        })
        .detach();
    }

    fn start_active_monitor_poll(&self, cx: &mut Context<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if state.active_monitor_polling {
                return;
            }
            state.active_monitor_polling = true;
        }
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(50))
                        .await;
                    let keep_polling = this
                        .update(&mut cx, |view, cx| view.apply_active_monitor_poll(cx))
                        .unwrap_or(false);
                    if !keep_polling {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn apply_active_monitor_poll(&mut self, cx: &mut Context<Self>) -> bool {
        if self.state.borrow().tx.is_none() {
            self.state.borrow_mut().active_monitor_polling = false;
            return false;
        }
        if self.sync_active_bounds_for_render() {
            self.notify_all(cx);
        }
        true
    }

    fn apply_global_pointer_sample(
        &mut self,
        position: Option<Point<Pixels>>,
        pressed: bool,
        cx: &mut Context<Self>,
    ) -> GlobalDragResult {
        if self.state.borrow().tx.is_none() {
            self.state.borrow_mut().polling = false;
            return GlobalDragResult::Stop;
        }
        if self.state.borrow().drag_start.is_none() {
            self.state.borrow_mut().polling = false;
            return GlobalDragResult::Stop;
        }
        let active_changed = self.sync_active_bounds_for_render();
        if let Some(position) = position {
            {
                let mut state = self.state.borrow_mut();
                state.drag_current = Some(position);
                state.set_active_bounds_for_point(position);
            }
            self.notify_all(cx);
        } else if active_changed {
            self.notify_all(cx);
        }
        if pressed {
            return GlobalDragResult::Continue;
        }
        let raw = self.current_raw_rect(position);
        let rect = raw.and_then(|rect| (self.map_rect)(rect));
        trace_selection_release("global", raw, rect);
        self.finish_from_global_pointer(rect)
    }

    fn current_raw_rect(&self, end: Option<Point<Pixels>>) -> Option<Rect> {
        let state = self.state.borrow();
        let end = end.or(state.drag_current)?;
        state
            .drag_start
            .and_then(|start| selected_rect(point(px(0.0), px(0.0)), start, end))
    }

    fn finish_from_global_pointer(&mut self, rect: Option<Rect>) -> GlobalDragResult {
        let Some(completion) = self.take_completion(rect) else {
            return GlobalDragResult::Stop;
        };
        self.hide_all_before_remove();
        GlobalDragResult::Finish {
            handles: self.state.borrow().handles.clone(),
            completion,
        }
    }

    fn take_completion(&mut self, rect: Option<Rect>) -> Option<SelectionCompletion> {
        let tx = {
            let mut state = self.state.borrow_mut();
            state.polling = false;
            state.tx.take()
        }?;
        match rect {
            Some(rect) => {
                qol_runtime::probe!("SHOT_SELECT_DONE", "path=gpui raw={}x{}", rect.w, rect.h)
            }
            None => qol_runtime::probe!("SHOT_SELECT_CANCEL", "path=gpui"),
        }
        Some(SelectionCompletion {
            tx,
            rect,
            quit_on_finish: self.quit_on_finish,
        })
    }

    fn remove_peer_windows(&self, cx: &mut Context<Self>) {
        let Some(handle) = self.handle else {
            return;
        };
        let handles = self.state.borrow().handles.clone();
        for peer in handles {
            if peer == handle {
                continue;
            }
            let _ = peer.update(cx, |view, window, _cx| {
                view.hide_window_before_remove(window);
                window.remove_window();
            });
        }
    }

    fn hide_all_before_remove(&self) {
        let _reason = qol_gpui::popup_window::reason_scope("shot-selector-finish");
        let titles = self.state.borrow().titles.clone();
        qol_runtime::probe!(
            "SHOT_SELECT_HIDE_ALL",
            "titles={} prefix={SELECTOR_TITLE_PREFIX}",
            titles.len()
        );
        for title in titles {
            let hidden = qol_gpui::popup_window::hide_invisible(&title);
            qol_runtime::probe!(
                "SHOT_SELECT_WINDOW",
                "title={title} state=hide-before-remove hidden={hidden}"
            );
        }
        let hidden_by_prefix =
            qol_gpui::popup_window::hide_windows_by_title_prefix(SELECTOR_TITLE_PREFIX);
        qol_runtime::probe!(
            "SHOT_SELECT_HIDE_ALL",
            "prefix={SELECTOR_TITLE_PREFIX} hidden_by_prefix={hidden_by_prefix}"
        );
    }

    fn hide_window_before_remove(&self, window: &mut Window) -> bool {
        let _reason = qol_gpui::popup_window::reason_scope("shot-selector-finish");
        let hidden = qol_gpui::popup_window::hide_for_capture(&self.title, window);
        qol_runtime::probe!(
            "SHOT_SELECT_WINDOW",
            "title={} state=hide-handle hidden={hidden}",
            self.title
        );
        hidden
    }

    fn guide_title(&self) -> &'static str {
        if self.state.borrow().drag_start.is_some() {
            return "Release mouse to capture";
        }
        "Drag to select capture area"
    }

    fn selection_bounds(&self, window: &Window) -> Option<Bounds<Pixels>> {
        let state = self.state.borrow();
        let start = state.drag_start?;
        let current = state.drag_current?;
        let left = start.x.min(current.x);
        let top = start.y.min(current.y);
        let right = start.x.max(current.x);
        let bottom = start.y.max(current.y);
        let selection = Bounds::new(point(left, top), size(right - left, bottom - top));
        let window_bounds = window.bounds();
        let clipped = intersect_bounds(selection, window_bounds)?;
        Some(Bounds::new(
            point(
                clipped.origin.x - window_bounds.origin.x,
                clipped.origin.y - window_bounds.origin.y,
            ),
            clipped.size,
        ))
    }

    fn guide_frame(&self, window: &Window) -> Option<(f32, f32, f32)> {
        let window_bounds = window.bounds();
        let bounds = self.state.borrow().active_bounds.unwrap_or(window_bounds);
        intersect_bounds(bounds, window_bounds)?;
        let local_x = f32::from(bounds.origin.x) - f32::from(window_bounds.origin.x);
        let local_y = f32::from(bounds.origin.y) - f32::from(window_bounds.origin.y);
        let monitor_width = f32::from(bounds.size.width);
        let guide_width = (monitor_width - GUIDE_MARGIN_X * 2.0).clamp(1.0, GUIDE_W);
        let guide_left = local_x + (monitor_width - guide_width) / 2.0;
        Some((guide_left, local_y + GUIDE_TOP, guide_width))
    }

    fn global_point(&self, local: Point<Pixels>, window: &Window) -> Point<Pixels> {
        let window_origin = window.bounds().origin;
        point(window_origin.x + local.x, window_origin.y + local.y)
    }

    fn sync_active_bounds(&self) -> bool {
        let Some(monitor) = qol_gpui::ghost::resolve_active_monitor() else {
            return false;
        };
        let bounds = monitor.bounds();
        self.state.borrow_mut().set_active_bounds(bounds)
    }

    fn sync_active_bounds_for_render(&self) -> bool {
        let current = self.state.borrow().drag_current;
        if let Some(current) = current {
            return self.state.borrow_mut().set_active_bounds_for_point(current);
        }
        self.sync_active_bounds()
    }

    fn notify_all(&self, cx: &mut Context<Self>) {
        cx.notify();
        let Some(handle) = self.handle else {
            return;
        };
        let handles = self.state.borrow().handles.clone();
        for peer in handles {
            if peer == handle {
                continue;
            }
            let _ = peer.update(cx, |_view, _window, cx| cx.notify());
        }
    }
}

fn schedule_completion(completion: SelectionCompletion, cx: &mut Context<RegionSelector>) {
    cx.spawn(
        move |_this: WeakEntity<RegionSelector>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                wait_for_selector_hide_barrier(&mut cx).await;
                let _ = cx.update(move |cx| cx.defer(move |cx| completion.finish(cx)));
            }
        },
    )
    .detach();
}

async fn wait_for_selector_hide_barrier(cx: &mut AsyncApp) {
    let started = std::time::Instant::now();
    let mut clear_samples = 0;

    loop {
        cx.background_executor()
            .timer(Duration::from_millis(16))
            .await;
        let visible = cx
            .update(|_| {
                qol_gpui::popup_window::visible_windows_by_title_prefix(SELECTOR_TITLE_PREFIX)
            })
            .unwrap_or(0);
        if visible == 0 {
            clear_samples += 1;
            if clear_samples >= HIDE_BARRIER_CLEAR_SAMPLES {
                qol_runtime::probe!(
                    "SHOT_SELECT_BARRIER",
                    "result=clear samples={} ms={}",
                    clear_samples,
                    started.elapsed().as_millis()
                );
                return;
            }
        } else {
            clear_samples = 0;
        }

        if started.elapsed() >= Duration::from_millis(HIDE_BARRIER_MAX_MS) {
            qol_runtime::probe!(
                "SHOT_SELECT_BARRIER",
                "result=timeout visible={} samples={} ms={}",
                visible,
                clear_samples,
                started.elapsed().as_millis()
            );
            return;
        }
    }
}

impl Focusable for RegionSelector {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RegionSelector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let guide_frame = self.guide_frame(window);
        let selection = self.selection_bounds(window);
        let mut root = div()
            .id("shot-region-selector")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .cursor(CursorStyle::Crosshair)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up));

        for bounds in backdrop_segments(window.bounds().size, selection) {
            root = root.child(backdrop_segment(bounds));
        }

        if let Some((guide_left, guide_top, guide_width)) = guide_frame {
            root = root.child(
                OverlayText {
                    title: self.guide_title(),
                    subtitle: Some("Press Esc to cancel"),
                    title_size: 22.0,
                    subtitle_size: 14.0,
                }
                .panel(guide_left, guide_top, guide_width, GUIDE_H),
            );
        }

        if let Some(bounds) = selection {
            root = root.child(selection_frame(bounds));
            if guide_frame.is_some()
                && bounds.size.width >= px(LABEL_MIN_W)
                && bounds.size.height >= px(LABEL_MIN_H)
            {
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

fn trace_selection_release(source: &'static str, raw: Option<Rect>, mapped: Option<Rect>) {
    let raw = raw
        .map(|rect| format!("{}x{}+{},{}", rect.w, rect.h, rect.x, rect.y))
        .unwrap_or_else(|| "none".to_string());
    let mapped = mapped
        .map(|rect| format!("{}x{}+{},{}", rect.w, rect.h, rect.x, rect.y))
        .unwrap_or_else(|| "none".to_string());
    qol_runtime::probe!(
        "SHOT_SELECT_RELEASE",
        "source={source} raw={raw} mapped={mapped}"
    );
}

fn backdrop_segment(bounds: Bounds<Pixels>) -> Div {
    div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .bg(rgba(BACKDROP_COLOR))
}

fn selection_frame(bounds: Bounds<Pixels>) -> Div {
    let mut frame = div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .border_2()
        .border_color(rgb(0xffffff));

    if bounds.size.width <= px(4.0) || bounds.size.height <= px(4.0) {
        return frame;
    }

    frame = frame.child(
        div()
            .absolute()
            .left(px(2.0))
            .top(px(2.0))
            .w(bounds.size.width - px(4.0))
            .h(bounds.size.height - px(4.0))
            .border_2()
            .border_color(rgb(0xff4d4d)),
    );
    frame
}

fn backdrop_segments(
    window_size: Size<Pixels>,
    selection: Option<Bounds<Pixels>>,
) -> Vec<Bounds<Pixels>> {
    let full = Bounds::new(point(px(0.0), px(0.0)), window_size);
    let Some(selection) = selection.and_then(|selection| intersect_bounds(selection, full)) else {
        return vec![full];
    };

    let window_w = window_size.width.to_f64();
    let window_h = window_size.height.to_f64();
    let left = selection.origin.x.to_f64().clamp(0.0, window_w);
    let top = selection.origin.y.to_f64().clamp(0.0, window_h);
    let right = (selection.origin.x + selection.size.width)
        .to_f64()
        .clamp(0.0, window_w);
    let bottom = (selection.origin.y + selection.size.height)
        .to_f64()
        .clamp(0.0, window_h);
    let selection_h = bottom - top;

    let mut segments = Vec::with_capacity(4);
    push_segment(&mut segments, 0.0, 0.0, window_w, top);
    push_segment(&mut segments, 0.0, top, left, selection_h);
    push_segment(&mut segments, right, top, window_w - right, selection_h);
    push_segment(&mut segments, 0.0, bottom, window_w, window_h - bottom);
    segments
}

fn push_segment(segments: &mut Vec<Bounds<Pixels>>, x: f64, y: f64, w: f64, h: f64) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    segments.push(Bounds::new(
        point(px(x as f32), px(y as f32)),
        size(px(w as f32), px(h as f32)),
    ));
}

impl Drop for RegionSelector {
    fn drop(&mut self) {
        if let Some(tx) = self.state.borrow_mut().tx.take() {
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

fn intersect_bounds(left: Bounds<Pixels>, right: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
    let x = left.origin.x.to_f64().max(right.origin.x.to_f64());
    let y = left.origin.y.to_f64().max(right.origin.y.to_f64());
    let right_edge = (left.origin.x + left.size.width)
        .to_f64()
        .min((right.origin.x + right.size.width).to_f64());
    let bottom_edge = (left.origin.y + left.size.height)
        .to_f64()
        .min((right.origin.y + right.size.height).to_f64());
    let width = right_edge - x;
    let height = bottom_edge - y;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(Bounds::new(
        point(px(x as f32), px(y as f32)),
        size(px(width as f32), px(height as f32)),
    ))
}

fn monitor_bounds_for_point(
    monitors: &[Bounds<Pixels>],
    point: Point<Pixels>,
) -> Option<Bounds<Pixels>> {
    monitors
        .iter()
        .copied()
        .find(|bounds| bounds_contains_point(*bounds, point))
}

fn bounds_contains_point(bounds: Bounds<Pixels>, point: Point<Pixels>) -> bool {
    point.x >= bounds.origin.x
        && point.x < bounds.origin.x + bounds.size.width
        && point.y >= bounds.origin.y
        && point.y < bounds.origin.y + bounds.size.height
}

#[cfg(test)]
mod tests {
    use super::{
        backdrop_segments, intersect_bounds, monitor_bounds_for_point, selected_rect,
        SelectionState,
    };
    use crate::Rect;
    use gpui::{point, px, size, Bounds};
    use std::sync::mpsc;

    #[test]
    fn selected_rect_adds_window_origin() {
        assert_eq!(
            selected_rect(
                point(px(100.0), px(50.0)),
                point(px(30.0), px(40.0)),
                point(px(10.0), px(90.0))
            ),
            Some(Rect {
                x: 110,
                y: 90,
                w: 20,
                h: 50,
            })
        );
    }

    #[test]
    fn selected_rect_rejects_empty_selection() {
        assert_eq!(
            selected_rect(
                point(px(0.0), px(0.0)),
                point(px(8.0), px(8.0)),
                point(px(8.0), px(12.0))
            ),
            None
        );
    }

    #[test]
    fn intersect_bounds_returns_overlap() {
        assert_eq!(
            intersect_bounds(
                Bounds::new(point(px(10.0), px(20.0)), size(px(100.0), px(80.0))),
                Bounds::new(point(px(50.0), px(10.0)), size(px(80.0), px(60.0)))
            ),
            Some(Bounds::new(
                point(px(50.0), px(20.0)),
                size(px(60.0), px(50.0))
            ))
        );
    }

    #[test]
    fn monitor_bounds_for_point_handles_vertically_offset_displays() {
        let laptop = Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let external = Bounds::new(point(px(-1512.0), px(458.0)), size(px(1512.0), px(982.0)));
        assert_eq!(
            monitor_bounds_for_point(&[laptop, external], point(px(-800.0), px(700.0))),
            Some(external)
        );
        assert_eq!(
            monitor_bounds_for_point(&[laptop, external], point(px(800.0), px(700.0))),
            Some(laptop)
        );
        assert_eq!(
            monitor_bounds_for_point(&[laptop, external], point(px(-800.0), px(200.0))),
            None
        );
    }

    #[test]
    fn active_bounds_follow_drag_pointer_monitor() {
        let (tx, _rx) = mpsc::channel();
        let laptop = Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let external = Bounds::new(point(px(-1512.0), px(458.0)), size(px(1512.0), px(982.0)));
        let mut state = SelectionState::new(tx, Some(laptop), vec![laptop, external], Vec::new());

        assert!(state.set_active_bounds_for_point(point(px(-800.0), px(700.0))));
        assert_eq!(state.active_bounds, Some(external));
        assert!(!state.set_active_bounds_for_point(point(px(-800.0), px(200.0))));
        assert_eq!(state.active_bounds, Some(external));
    }

    #[test]
    fn backdrop_segments_dim_whole_window_without_selection() {
        assert_eq!(
            backdrop_segments(size(px(100.0), px(80.0)), None),
            vec![Bounds::new(
                point(px(0.0), px(0.0)),
                size(px(100.0), px(80.0))
            )]
        );
    }

    #[test]
    fn backdrop_segments_leave_middle_selection_clear() {
        assert_eq!(
            backdrop_segments(
                size(px(100.0), px(80.0)),
                Some(Bounds::new(
                    point(px(20.0), px(10.0)),
                    size(px(50.0), px(40.0))
                )),
            ),
            vec![
                Bounds::new(point(px(0.0), px(0.0)), size(px(100.0), px(10.0))),
                Bounds::new(point(px(0.0), px(10.0)), size(px(20.0), px(40.0))),
                Bounds::new(point(px(70.0), px(10.0)), size(px(30.0), px(40.0))),
                Bounds::new(point(px(0.0), px(50.0)), size(px(100.0), px(30.0))),
            ]
        );
    }

    #[test]
    fn backdrop_segments_skip_zero_sized_edge_bands() {
        assert_eq!(
            backdrop_segments(
                size(px(100.0), px(80.0)),
                Some(Bounds::new(
                    point(px(0.0), px(0.0)),
                    size(px(40.0), px(40.0))
                )),
            ),
            vec![
                Bounds::new(point(px(40.0), px(0.0)), size(px(60.0), px(40.0))),
                Bounds::new(point(px(0.0), px(40.0)), size(px(100.0), px(40.0))),
            ]
        );
    }
}
