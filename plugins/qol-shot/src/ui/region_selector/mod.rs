use anyhow::Result;
use gpui::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, LazyLock};
use std::time::Duration;

use crate::capture::frozen_frame::FrozenFrame;
use crate::capture::geometry::rect_label;
use crate::capture::space::{self, CaptureKind, Level};
use crate::{Monitor, Rect};
use qol_gpui::placement::{intersect_bounds, monitor_at_point, project_bounds, MonitorPlacement};
use qol_gpui::theme::{shot_selector_runtime, ShotSelectorPalette};
use qol_gpui::toast::{Toast, ToastLayout, ToastTone};

const SELECTOR_TITLE: &str = "qol-shot-selector";
pub(crate) const SELECTOR_TITLE_PREFIX: &str = "qol-shot-selector-";
const SELECTOR_APP_ID: &str = "qol-tray-shot";
const LABEL_MIN_W: f32 = 180.0;
const LABEL_MIN_H: f32 = 80.0;
const CHIP_W: f32 = 300.0;
const CHIP_H: f32 = 30.0;
const CHIP_TOP: f32 = 12.0;
const SELECTOR_STATE_POLL_MS: u64 = 16;
static SELECTOR_SEQ: AtomicU64 = AtomicU64::new(0);
static CURRENT_PALETTE: LazyLock<ShotSelectorPalette> = LazyLock::new(shot_selector_runtime);

fn current_palette() -> &'static ShotSelectorPalette {
    &CURRENT_PALETTE
}

pub type RectMapper = Rc<dyn Fn(Rect) -> Option<Rect>>;

pub trait ActiveBounds {
    fn active_bounds(&self) -> Option<Bounds<Pixels>>;
}

pub type ActiveBoundsSource = Rc<dyn ActiveBounds>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(target_os = "linux"),
    allow(
        dead_code,
        reason = "only the linux backend constructs hover-detected targets"
    )
)]
pub enum DetectedTarget {
    Window(Rect),
    Monitor(Rect),
}

impl DetectedTarget {
    pub fn rect(self) -> Rect {
        match self {
            Self::Window(rect) | Self::Monitor(rect) => rect,
        }
    }
}

pub trait HoverTarget {
    fn target_at(&self, point: Point<Pixels>) -> Option<DetectedTarget>;
}

pub type HoverTargetSource = Rc<dyn HoverTarget>;

pub trait GlobalPointer {
    fn position(&self) -> Option<Point<Pixels>>;
    fn primary_button_down(&self) -> bool;
}

pub type GlobalPointerSource = Rc<dyn GlobalPointer>;
pub type CancelSignalSource = Rc<dyn Fn() -> bool>;
pub type SelectorReveal = Rc<dyn Fn(String)>;

#[derive(Clone)]
pub struct SelectorWindowSources {
    pub map_rect: RectMapper,
    pub global_pointer: Option<GlobalPointerSource>,
    pub cancel_signal: Option<CancelSignalSource>,
    pub active_bounds: Option<ActiveBoundsSource>,
    pub hover_target: Option<HoverTargetSource>,
    pub frozen_frame: Option<FrozenFrame>,
}

pub struct SelectorWindowOptions {
    pub display_id: Option<DisplayId>,
    pub kind: WindowKind,
    pub decorations: WindowDecorations,
    pub focus: bool,
}

pub struct SelectorWindow {
    title: String,
    bounds: Bounds<Pixels>,
    monitor_bounds: Vec<Bounds<Pixels>>,
    active_bounds: Option<Bounds<Pixels>>,
    default_target: Option<DetectedTarget>,
    display_id: Option<DisplayId>,
    kind: WindowKind,
    decorations: WindowDecorations,
    focus: bool,
    sources: SelectorWindowSources,
}

impl SelectorWindow {
    pub fn new(
        bounds: Bounds<Pixels>,
        monitor_bounds: Vec<Bounds<Pixels>>,
        active_bounds: Option<Bounds<Pixels>>,
        default_target: Option<DetectedTarget>,
        options: SelectorWindowOptions,
        sources: SelectorWindowSources,
    ) -> Self {
        Self {
            title: selector_title(),
            bounds,
            monitor_bounds,
            active_bounds,
            default_target,
            display_id: options.display_id,
            kind: options.kind,
            decorations: options.decorations,
            focus: options.focus,
            sources,
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

pub(crate) mod platform;

pub fn open_all(
    tx: mpsc::Sender<Option<Rect>>,
    quit_on_finish: bool,
    selectors: Vec<SelectorWindow>,
    kind: CaptureKind,
    cx: &mut App,
) -> bool {
    let selector_count = selectors.len();
    let active_bounds = selectors.iter().find_map(|selector| selector.active_bounds);
    let default_target = selectors
        .iter()
        .find_map(|selector| selector.default_target);
    let monitor_bounds = selectors
        .iter()
        .flat_map(|selector| selector.monitor_bounds.iter().copied())
        .fold(Vec::new(), |mut monitors, bounds| {
            if !monitors.contains(&bounds) {
                monitors.push(bounds);
            }
            monitors
        });
    let titles = selectors
        .iter()
        .map(|selector| selector.title.clone())
        .collect::<Vec<_>>();
    let state = Rc::new(RefCell::new(SelectionState::new(
        tx,
        active_bounds,
        default_target,
        monitor_bounds,
        titles,
        kind,
    )));
    let mut handles = Vec::new();
    for selector in selectors {
        let Some(handle) = open_window(selector, state.clone(), quit_on_finish, false, true, cx)
        else {
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
    reusable: bool,
    show: bool,
    cx: &mut App,
) -> Option<WindowHandle<RegionSelector>> {
    let mut options = selector.options();
    options.show = show;
    options.focus = show && selector.focus && !reusable;
    let focus = options.focus;
    let window_bounds = selector.bounds;
    let sources = selector.sources;
    let window_title = selector.title;
    cx.open_window(options, move |window, cx| {
        window.set_window_title(&window_title);
        qol_runtime::probe!("SHOT_SELECT_WINDOW", "title={window_title} state=open");
        state.borrow_mut().record_display(
            rect_from_bounds(window_bounds),
            window.scale_factor() as f64,
        );
        let view = cx.new(|cx| {
            RegionSelector::new(
                state,
                window_title.clone(),
                quit_on_finish,
                window_bounds,
                sources,
                reusable,
                cx,
            )
        });
        if show && focus {
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

fn selector_title() -> String {
    let seq = SELECTOR_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{seq}", SELECTOR_TITLE, std::process::id())
}

#[derive(Clone, Copy)]
struct ChipModel {
    kind: CaptureKind,
    free_bytes: u64,
    fps: u32,
    audio: bool,
    quality: space::Quality,
}

impl ChipModel {
    fn load(kind: CaptureKind) -> Self {
        let config = crate::config::load();
        let free_bytes = qol_platform::launch_working_dir()
            .or_else(|| Some(std::env::temp_dir()))
            .and_then(|dir| qol_platform::disk_space(&dir).ok())
            .map(|space| space.available)
            .unwrap_or(0);
        Self {
            kind,
            free_bytes,
            fps: config.video.framerate,
            audio: config.audio.enabled,
            quality: space::Quality::from_crf(config.video.crf),
        }
    }

    fn estimate_for(&self, pixels: u64) -> space::Estimate {
        space::estimate(&space::Capture::Video {
            pixels,
            fps: self.fps,
            audio: self.audio,
            quality: self.quality,
        })
    }
}

struct SelectionState {
    tx: Option<mpsc::Sender<Option<Rect>>>,
    active_bounds: Option<Bounds<Pixels>>,
    default_target: Option<DetectedTarget>,
    monitor_bounds: Vec<Bounds<Pixels>>,
    titles: Vec<String>,
    drag_start: Option<Point<Pixels>>,
    drag_current: Option<Point<Pixels>>,
    pointer_offset: Option<Point<Pixels>>,
    handles: Vec<WindowHandle<RegionSelector>>,
    polling: bool,
    active_monitor_polling: bool,
    chip: ChipModel,
    displays: Vec<space::DisplayScale>,
}

impl SelectionState {
    fn new(
        tx: mpsc::Sender<Option<Rect>>,
        active_bounds: Option<Bounds<Pixels>>,
        default_target: Option<DetectedTarget>,
        monitor_bounds: Vec<Bounds<Pixels>>,
        titles: Vec<String>,
        kind: CaptureKind,
    ) -> Self {
        Self {
            tx: Some(tx),
            active_bounds,
            default_target,
            monitor_bounds,
            titles,
            drag_start: None,
            drag_current: None,
            pointer_offset: None,
            handles: Vec::new(),
            polling: false,
            active_monitor_polling: false,
            chip: ChipModel::load(kind),
            displays: Vec::new(),
        }
    }

    fn record_display(&mut self, bounds: Rect, scale: f64) {
        if self.displays.iter().any(|display| display.bounds == bounds) {
            return;
        }
        self.displays.push(space::DisplayScale { bounds, scale });
    }

    fn set_active_bounds(&mut self, bounds: Bounds<Pixels>) -> bool {
        if self.active_bounds == Some(bounds) {
            return false;
        }
        self.active_bounds = Some(bounds);
        true
    }

    fn set_active_bounds_for_point(&mut self, point: Point<Pixels>) -> bool {
        let Some(bounds) = monitor_at_point(&self.monitor_bounds, point) else {
            return false;
        };
        self.set_active_bounds(bounds)
    }

    fn set_default_target(&mut self, target: Option<DetectedTarget>) -> bool {
        if self.default_target == target {
            return false;
        }
        self.default_target = target;
        true
    }

    fn resync_active_bounds(
        &mut self,
        pointer: Option<Point<Pixels>>,
        active_bounds: Option<Bounds<Pixels>>,
    ) -> bool {
        if let Some(current) = self.drag_current {
            return self.set_active_bounds_for_point(current);
        }
        if let Some(bounds) = active_bounds {
            return self.set_active_bounds(bounds);
        }
        let Some(position) = pointer else {
            return false;
        };
        self.set_active_bounds_for_point(position)
    }

    fn apply_gpui_drag(&mut self, position: Point<Pixels>) -> bool {
        if self.polling {
            return false;
        }
        if self.drag_start.is_none() {
            return false;
        }
        self.drag_current = Some(position);
        true
    }
}

enum SelectorPollResult {
    Continue,
    Stop,
    Finish {
        handles: Vec<WindowHandle<RegionSelector>>,
        completion: SelectionCompletion,
        retain_windows: bool,
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
    window_bounds: Bounds<Pixels>,
    map_rect: RectMapper,
    global_pointer: Option<GlobalPointerSource>,
    cancel_signal: Option<CancelSignalSource>,
    active_bounds: Option<ActiveBoundsSource>,
    hover_target: Option<HoverTargetSource>,
    frozen_image: Option<Arc<RenderImage>>,
    focus_handle: FocusHandle,
    reusable: bool,
    reveal_generation: u64,
    scheduled_reveal_generation: Option<u64>,
    pending_reveal: Option<SelectorReveal>,
}

impl RegionSelector {
    fn new(
        state: Rc<RefCell<SelectionState>>,
        title: String,
        quit_on_finish: bool,
        window_bounds: Bounds<Pixels>,
        sources: SelectorWindowSources,
        reusable: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        qol_runtime::probe!("SHOT_SELECT_START", "path=gpui");
        let image_started = std::time::Instant::now();
        let frozen_image = sources
            .frozen_frame
            .as_ref()
            .and_then(|frame| frame.render_image(rect_from_bounds(window_bounds)));
        qol_runtime::probe!(
            "SHOT_FREEZE_IMAGE",
            "ms={} ready={}",
            image_started.elapsed().as_millis(),
            frozen_image.is_some()
        );
        Self {
            state,
            handle: None,
            title,
            quit_on_finish,
            window_bounds,
            map_rect: sources.map_rect,
            global_pointer: sources.global_pointer,
            cancel_signal: sources.cancel_signal,
            active_bounds: sources.active_bounds,
            hover_target: sources.hover_target,
            frozen_image,
            focus_handle: cx.focus_handle(),
            reusable,
            reveal_generation: 0,
            scheduled_reveal_generation: None,
            pending_reveal: None,
        }
    }

    fn schedule_reveal_after_present(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.reveal_generation;
        if self.pending_reveal.is_none() || self.scheduled_reveal_generation == Some(generation) {
            return;
        }
        self.scheduled_reveal_generation = Some(generation);
        qol_runtime::probe!(
            "SHOT_SELECT_REVEAL",
            "title={} generation={generation} state=scheduled",
            self.title
        );
        cx.on_next_frame(window, move |view, _window, _cx| {
            view.reveal_presented_generation(generation);
        });
    }

    fn reveal_presented_generation(&mut self, generation: u64) {
        if generation != self.reveal_generation {
            qol_runtime::probe!(
                "SHOT_SELECT_REVEAL",
                "title={} generation={generation} current={} state=stale",
                self.title,
                self.reveal_generation
            );
            return;
        }
        self.scheduled_reveal_generation = None;
        let Some(reveal) = self.pending_reveal.take() else {
            return;
        };
        qol_runtime::probe!(
            "SHOT_SELECT_REVEAL",
            "title={} generation={generation} state=presented",
            self.title
        );
        reveal(self.title.clone());
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = self.global_point(event.position);
        let cg = self
            .global_pointer
            .as_ref()
            .and_then(|pointer| pointer.position());
        {
            let mut state = self.state.borrow_mut();
            state.drag_start = Some(position);
            state.drag_current = Some(position);
            state.pointer_offset = cg.map(|cg| point(cg.x - position.x, cg.y - position.y));
            state.set_active_bounds_for_point(position);
        }
        let monitor = self.state.borrow().active_bounds;
        let guide = self.guide_bounds();
        trace_drag_anchor(&self.title, event.position, position, cg, monitor, guide);
        self.notify_all(cx);
        self.start_global_drag(cx);
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = self.global_point(event.position);
        if event.dragging() {
            if self.state.borrow_mut().apply_gpui_drag(position) {
                self.notify_all(cx);
            }
            return;
        }
        let hover_changed = match &self.hover_target {
            Some(source) => {
                let target = source.target_at(position);
                let changed = self.state.borrow_mut().set_default_target(target);
                if changed {
                    trace_hover_target(target);
                }
                changed
            }
            None => false,
        };
        let active_bounds = self.active_bounds_sample();
        let resynced = self
            .state
            .borrow_mut()
            .resync_active_bounds(Some(position), active_bounds);
        if hover_changed || resynced {
            self.notify_all(cx);
        }
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let end = self.release_point(event.position);
        {
            let mut state = self.state.borrow_mut();
            state.drag_current = Some(end);
            state.set_active_bounds_for_point(end);
        }
        let raw = self.current_raw_rect(Some(end));
        let rect = self.resolved_capture_rect(raw);
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
        self.frozen_image = None;
        if !self.reusable {
            self.remove_peer_windows(cx);
            window.remove_window();
        }
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
                        .unwrap_or(SelectorPollResult::Stop);
                    if !continue_selector_poll(result, &mut cx).await {
                        break;
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
                        .timer(Duration::from_millis(SELECTOR_STATE_POLL_MS))
                        .await;
                    let result = this
                        .update(&mut cx, |view, cx| view.apply_active_monitor_poll(cx))
                        .unwrap_or(SelectorPollResult::Stop);
                    if !continue_selector_poll(result, &mut cx).await {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn apply_active_monitor_poll(&mut self, cx: &mut Context<Self>) -> SelectorPollResult {
        if self.state.borrow().tx.is_none() {
            self.state.borrow_mut().active_monitor_polling = false;
            return SelectorPollResult::Stop;
        }
        if self.cancel_signal.as_ref().is_some_and(|cancel| cancel()) {
            self.state.borrow_mut().active_monitor_polling = false;
            qol_runtime::probe!("SHOT_SELECT_CANCEL_INPUT", "source=global-escape");
            return self.finish_from_poll(None);
        }
        if self.sync_active_bounds_for_render() {
            self.notify_all(cx);
        }
        SelectorPollResult::Continue
    }

    fn apply_global_pointer_sample(
        &mut self,
        position: Option<Point<Pixels>>,
        pressed: bool,
        cx: &mut Context<Self>,
    ) -> SelectorPollResult {
        if self.state.borrow().tx.is_none() {
            self.state.borrow_mut().polling = false;
            return SelectorPollResult::Stop;
        }
        if self.state.borrow().drag_start.is_none() {
            self.state.borrow_mut().polling = false;
            return SelectorPollResult::Stop;
        }
        let tracked = position.map(|cg| self.tracked_point(cg));
        let active_changed = self.sync_active_bounds_for_render();
        if let Some(tracked) = tracked {
            {
                let mut state = self.state.borrow_mut();
                state.drag_current = Some(tracked);
                state.set_active_bounds_for_point(tracked);
            }
            self.notify_all(cx);
        } else if active_changed {
            self.notify_all(cx);
        }
        if pressed {
            return SelectorPollResult::Continue;
        }
        let raw = self.current_raw_rect(tracked);
        let rect = self.resolved_capture_rect(raw);
        trace_selection_release("global", raw, rect);
        self.finish_from_poll(rect)
    }

    fn tracked_point(&self, global: Point<Pixels>) -> Point<Pixels> {
        local_from_global(global, self.state.borrow().pointer_offset)
    }

    fn release_point(&self, fallback_local: Point<Pixels>) -> Point<Pixels> {
        self.global_pointer
            .as_ref()
            .and_then(|pointer| pointer.position())
            .map(|cg| self.tracked_point(cg))
            .unwrap_or_else(|| self.global_point(fallback_local))
    }

    fn capture_rect(&self, raw: Option<Rect>) -> Option<Rect> {
        let offset = self.state.borrow().pointer_offset;
        raw.map(|rect| shift_rect(rect, offset))
            .and_then(|rect| (self.map_rect)(rect))
    }

    fn resolved_capture_rect(&self, raw: Option<Rect>) -> Option<Rect> {
        if raw.is_some() {
            return self.capture_rect(raw);
        }
        self.state.borrow().default_target.map(DetectedTarget::rect)
    }

    fn current_raw_rect(&self, end: Option<Point<Pixels>>) -> Option<Rect> {
        let state = self.state.borrow();
        let end = end.or(state.drag_current)?;
        let start = state.drag_start?;
        trace_drag_rect(start, end);
        selected_rect(point(px(0.0), px(0.0)), start, end)
    }

    fn finish_from_poll(&mut self, rect: Option<Rect>) -> SelectorPollResult {
        let Some(completion) = self.take_completion(rect) else {
            return SelectorPollResult::Stop;
        };
        self.hide_all_before_remove();
        self.frozen_image = None;
        SelectorPollResult::Finish {
            handles: self.state.borrow().handles.clone(),
            completion,
            retain_windows: self.reusable,
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
        match self.state.borrow().default_target {
            Some(DetectedTarget::Window(_)) => "Detected window",
            Some(DetectedTarget::Monitor(_)) => "Full monitor",
            None => "No window detected",
        }
    }

    fn guide_subtitle(&self) -> &'static str {
        if self.state.borrow().drag_start.is_some() {
            return "Press Esc to cancel";
        }
        if self.state.borrow().default_target.is_some() {
            return "Click to capture or drag to select area";
        }
        "Drag to select area or press Esc to cancel"
    }

    fn selection_bounds(&self) -> Option<Bounds<Pixels>> {
        let state = self.state.borrow();
        selection_global_rect(&state)
            .and_then(|selection| selection_bounds_in_window(selection, self.window_bounds))
    }

    fn guide_bounds(&self) -> Option<Bounds<Pixels>> {
        let monitor = self
            .state
            .borrow()
            .active_bounds
            .unwrap_or(self.window_bounds);
        ToastLayout::Status.placement().projected_bounds(
            monitor,
            ToastLayout::Status.size(),
            self.window_bounds,
        )
    }

    fn chip_bounds(&self) -> Option<Bounds<Pixels>> {
        let monitor = self.state.borrow().active_bounds?;
        MonitorPlacement::top_center(CHIP_TOP).projected_bounds(
            monitor,
            size(px(CHIP_W), px(CHIP_H)),
            self.window_bounds,
        )
    }

    fn chip_status(&self) -> (String, Level) {
        let state = self.state.borrow();
        let chip = state.chip;
        let label = kind_label(chip.kind);
        let free = format_bytes(chip.free_bytes);
        let estimate = capture_estimate(chip, selection_global_rect(&state), &state.displays);
        let headroom = space::headroom(&estimate, chip.free_bytes);
        let text = match headroom.seconds {
            Some(seconds) => format!("{label} · {free} free · ~{}", format_duration(seconds)),
            None => format!("{label} · {free} free"),
        };
        (text, headroom.level)
    }

    fn global_point(&self, local: Point<Pixels>) -> Point<Pixels> {
        let window_origin = self.window_bounds.origin;
        point(window_origin.x + local.x, window_origin.y + local.y)
    }

    fn sync_active_bounds_for_render(&self) -> bool {
        let pointer = self
            .global_pointer
            .as_ref()
            .and_then(|pointer| pointer.position())
            .map(|cg| self.tracked_point(cg));
        self.state
            .borrow_mut()
            .resync_active_bounds(pointer, self.active_bounds_sample())
    }

    fn active_bounds_sample(&self) -> Option<Bounds<Pixels>> {
        self.active_bounds
            .as_ref()
            .and_then(|active_bounds| active_bounds.active_bounds())
    }

    fn selection_label_title(&self) -> &'static str {
        let state = self.state.borrow();
        if manual_selection_global_rect(&state).is_some() {
            return "Capture area";
        }
        match state.default_target {
            Some(DetectedTarget::Window(_)) => "Detected window",
            Some(DetectedTarget::Monitor(_)) => "Full monitor",
            None => "Capture area",
        }
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

async fn continue_selector_poll(result: SelectorPollResult, cx: &mut AsyncApp) -> bool {
    match result {
        SelectorPollResult::Continue => true,
        SelectorPollResult::Stop => false,
        SelectorPollResult::Finish {
            handles,
            completion,
            retain_windows,
        } => {
            for handle in handles {
                let _ = handle.update(cx, |view, window, _cx| {
                    view.hide_window_before_remove(window);
                    if !retain_windows {
                        window.remove_window();
                    }
                });
            }
            wait_for_selector_hide_barrier(cx).await;
            let _ = cx.update(move |cx| cx.defer(move |cx| completion.finish(cx)));
            false
        }
    }
}

async fn wait_for_selector_hide_barrier(cx: &mut AsyncApp) {
    let barrier = qol_gpui::popup_window::wait_for_hidden_windows(cx, SELECTOR_TITLE_PREFIX).await;
    qol_runtime::probe!(
        "SHOT_SELECT_BARRIER",
        "result={} visible={} samples={} ms={}",
        if barrier.cleared { "clear" } else { "timeout" },
        barrier.visible,
        barrier.clear_samples,
        barrier.elapsed.as_millis()
    );
}

impl Focusable for RegionSelector {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RegionSelector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.schedule_reveal_after_present(window, cx);
        let guide_bounds = self.guide_bounds();
        let selection = self.selection_bounds();
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

        if let Some(image) = &self.frozen_image {
            root = root.child(
                img(image.clone())
                    .absolute()
                    .left(px(0.0))
                    .top(px(0.0))
                    .w(self.window_bounds.size.width)
                    .h(self.window_bounds.size.height),
            );
        }

        for bounds in backdrop_segments(self.window_bounds.size, selection) {
            root = root.child(backdrop_segment(bounds));
        }

        if let Some(bounds) = guide_bounds {
            let guide = Toast::new(self.guide_title(), self.guide_subtitle())
                .layout(ToastLayout::Status)
                .tone(ToastTone::Info);
            root = root.child(guide.positioned(bounds));
        }

        if let Some(bounds) = selection {
            root = root.child(selection_frame(bounds));
            if guide_bounds.is_some()
                && bounds.size.width >= px(LABEL_MIN_W)
                && bounds.size.height >= px(LABEL_MIN_H)
            {
                let label_top =
                    f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0 - 13.0;
                root = root.child(
                    SelectionLabel {
                        title: self.selection_label_title().into(),
                    }
                    .positioned(
                        f32::from(bounds.origin.x) + 12.0,
                        label_top,
                        f32::from(bounds.size.width) - 24.0,
                        26.0,
                    ),
                );
            }
        }

        if let Some(bounds) = self.chip_bounds() {
            let (text, level) = self.chip_status();
            root = root.child(chip_element(bounds, text, level));
        }

        root
    }
}

fn trace_hover_target(target: Option<DetectedTarget>) {
    #[cfg(debug_assertions)]
    {
        let target = match target {
            Some(DetectedTarget::Window(rect)) => format!("window:{}", rect_label(rect)),
            Some(DetectedTarget::Monitor(rect)) => format!("monitor:{}", rect_label(rect)),
            None => "none".to_string(),
        };
        qol_runtime::probe!("SHOT_SELECT_TARGET", "hover={target}");
    }
    #[cfg(not(debug_assertions))]
    let _ = target;
}

#[cfg(debug_assertions)]
fn fmt_pt(p: Point<Pixels>) -> String {
    format!("{},{}", f32::from(p.x) as i32, f32::from(p.y) as i32)
}

#[cfg(debug_assertions)]
fn fmt_bounds(bounds: Bounds<Pixels>) -> String {
    format!(
        "{}x{}+{},{}",
        f32::from(bounds.size.width) as i32,
        f32::from(bounds.size.height) as i32,
        f32::from(bounds.origin.x) as i32,
        f32::from(bounds.origin.y) as i32
    )
}

fn trace_drag_anchor(
    title: &str,
    local: Point<Pixels>,
    win_pt: Point<Pixels>,
    cg: Option<Point<Pixels>>,
    monitor: Option<Bounds<Pixels>>,
    guide: Option<Bounds<Pixels>>,
) {
    #[cfg(debug_assertions)]
    {
        let cg = cg.map(fmt_pt).unwrap_or_else(|| "none".to_string());
        let monitor = monitor
            .map(fmt_bounds)
            .unwrap_or_else(|| "none".to_string());
        let guide = guide.map(fmt_bounds).unwrap_or_else(|| "none".to_string());
        qol_runtime::probe!(
            "SHOT_DRAG_ANCHOR",
            "title={title} local={} win_pt={} cg_pt={cg} monitor={monitor} guide={guide}",
            fmt_pt(local),
            fmt_pt(win_pt)
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (title, local, win_pt, cg, monitor, guide);
}

fn trace_drag_rect(start: Point<Pixels>, end: Point<Pixels>) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "SHOT_DRAG_RECT",
        "start={} end={}",
        fmt_pt(start),
        fmt_pt(end)
    );
    #[cfg(not(debug_assertions))]
    let _ = (start, end);
}

fn trace_selection_release(source: &'static str, raw: Option<Rect>, mapped: Option<Rect>) {
    let raw = raw.map(rect_label).unwrap_or_else(|| "none".to_string());
    let mapped = mapped.map(rect_label).unwrap_or_else(|| "none".to_string());
    qol_runtime::probe!(
        "SHOT_SELECT_RELEASE",
        "source={source} raw={raw} mapped={mapped}"
    );
}

fn backdrop_segment(bounds: Bounds<Pixels>) -> Div {
    let palette = current_palette();
    div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .bg(rgba(palette.backdrop_rgba))
}

fn rect_from_bounds(bounds: Bounds<Pixels>) -> Rect {
    Rect {
        x: f32::from(bounds.origin.x) as i32,
        y: f32::from(bounds.origin.y) as i32,
        w: f32::from(bounds.size.width) as i32,
        h: f32::from(bounds.size.height) as i32,
    }
}

fn selection_global_rect(state: &SelectionState) -> Option<Rect> {
    if let Some(rect) = manual_selection_global_rect(state) {
        return Some(rect);
    }
    state.default_target.map(DetectedTarget::rect)
}

fn manual_selection_global_rect(state: &SelectionState) -> Option<Rect> {
    let (start, current) = state.drag_start.zip(state.drag_current)?;
    selected_rect(point(px(0.0), px(0.0)), start, current)
}

fn local_from_global(global: Point<Pixels>, offset: Option<Point<Pixels>>) -> Point<Pixels> {
    let Some(offset) = offset else {
        return global;
    };
    point(global.x - offset.x, global.y - offset.y)
}

fn shift_rect(rect: Rect, offset: Option<Point<Pixels>>) -> Rect {
    let Some(offset) = offset else {
        return rect;
    };
    Rect {
        x: rect.x + f32::from(offset.x) as i32,
        y: rect.y + f32::from(offset.y) as i32,
        ..rect
    }
}

fn capture_estimate(
    chip: ChipModel,
    selection: Option<Rect>,
    displays: &[space::DisplayScale],
) -> space::Estimate {
    let zero = space::Estimate {
        rate_bps: 0,
        fixed: 0,
    };
    if chip.kind == CaptureKind::Screenshot {
        return zero;
    }
    selection
        .map(|rect| space::captured_pixels(rect, displays))
        .filter(|pixels| *pixels > 0)
        .map(|pixels| chip.estimate_for(pixels))
        .unwrap_or(zero)
}

fn selection_bounds_in_window(
    selection: Rect,
    window_bounds: Bounds<Pixels>,
) -> Option<Bounds<Pixels>> {
    let selection = Bounds::new(
        point(px(selection.x as f32), px(selection.y as f32)),
        size(px(selection.w as f32), px(selection.h as f32)),
    );
    project_bounds(selection, window_bounds)
}

fn chip_element(bounds: Bounds<Pixels>, text: String, level: Level) -> Div {
    let palette = current_palette();
    let (border, foreground) = chip_colors(level);
    div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .rounded(px(CHIP_H / 2.0))
        .border_1()
        .border_color(rgba(border))
        .bg(rgba(palette.panel_bg_rgba))
        .flex()
        .items_center()
        .justify_center()
        .text_center()
        .text_size(px(13.0))
        .line_height(px(CHIP_H))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgba(foreground))
        .child(SharedString::from(text))
}

fn chip_colors(level: Level) -> (u32, u32) {
    let palette = current_palette();
    match level {
        Level::Ok => (palette.chip_ok_border_rgba, palette.chip_ok_text_rgba),
        Level::Low => (palette.chip_low_border_rgba, palette.chip_low_text_rgba),
        Level::Critical => (
            palette.chip_critical_border_rgba,
            palette.chip_critical_text_rgba,
        ),
    }
}

fn kind_label(kind: CaptureKind) -> &'static str {
    match kind {
        CaptureKind::Screenshot => "Picture",
        CaptureKind::Recording => "Video",
    }
}

fn format_bytes(bytes: u64) -> String {
    const GB: u64 = 1_000_000_000;
    const MB: u64 = 1_000_000;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{} MB", bytes / MB)
    } else {
        format!("{} KB", bytes / 1_000)
    }
}

fn format_duration(seconds: u64) -> String {
    if seconds >= 3_600 {
        format!("{} h {} min", seconds / 3_600, (seconds % 3_600) / 60)
    } else if seconds >= 60 {
        format!("{} min", seconds / 60)
    } else {
        format!("{} sec", seconds)
    }
}

fn selection_frame(bounds: Bounds<Pixels>) -> Div {
    let palette = current_palette();
    let mut frame = div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .border_2()
        .border_color(rgb(palette.selection_outer));

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
            .border_color(rgb(palette.selection_inner)),
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

struct SelectionLabel {
    title: SharedString,
}

impl SelectionLabel {
    fn positioned(self, left: f32, top: f32, width: f32, height: f32) -> impl IntoElement {
        let palette = current_palette();
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
            .text_size(px(18.0))
            .line_height(px(height))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgba(palette.label_text_rgba))
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

#[cfg(test)]
mod tests {
    use super::{
        backdrop_segments, capture_estimate, format_bytes, format_duration, kind_label,
        local_from_global, selected_rect, selection_bounds_in_window, selection_global_rect,
        shift_rect, ChipModel, DetectedTarget, SelectionState, CHIP_H, CHIP_TOP, CHIP_W,
    };
    use crate::capture::space::{CaptureKind, DisplayScale, Quality};
    use crate::Rect;
    use gpui::{point, px, size, Bounds};
    use qol_gpui::placement::{monitor_at_point, MonitorPlacement};
    use qol_gpui::toast::ToastLayout;
    use std::sync::mpsc;

    fn chip(kind: CaptureKind) -> ChipModel {
        ChipModel {
            kind,
            free_bytes: 10_000_000_000,
            fps: 30,
            audio: false,
            quality: Quality::High,
        }
    }

    #[test]
    fn drag_round_trips_between_global_tracking_and_capture_space() {
        let offset = Some(point(px(1.0), px(32.0)));
        let cg = point(px(2820.0), px(1062.0));
        assert_eq!(
            local_from_global(cg, offset),
            point(px(2819.0), px(1030.0)),
            "a CG pointer sample maps into the gpui tracking frame the drag started in"
        );
        let tracked = Rect {
            x: 2819,
            y: 1030,
            w: 261,
            h: 122,
        };
        assert_eq!(
            shift_rect(tracked, offset),
            Rect {
                x: 2820,
                y: 1062,
                w: 261,
                h: 122
            },
            "the tracked rect maps back to CG capture coordinates for screencapture"
        );
    }

    #[test]
    fn drag_coordinates_are_identity_without_a_global_pointer() {
        let cg = point(px(100.0), px(200.0));
        assert_eq!(local_from_global(cg, None), cg);
        let rect = Rect {
            x: 5,
            y: 6,
            w: 7,
            h: 8,
        };
        assert_eq!(shift_rect(rect, None), rect);
    }

    #[test]
    fn kind_label_names_the_capture_for_the_chip() {
        let cases = [
            (CaptureKind::Recording, "Video"),
            (CaptureKind::Screenshot, "Picture"),
        ];
        for (kind, expected) in cases {
            assert_eq!(kind_label(kind), expected, "kind: {kind:?}");
        }
    }

    #[test]
    fn capture_estimate_is_contextual_per_capture_kind() {
        let displays = [DisplayScale {
            bounds: Rect {
                x: 0,
                y: 0,
                w: 1000,
                h: 1000,
            },
            scale: 1.0,
        }];
        let selection = Some(Rect {
            x: 0,
            y: 0,
            w: 200,
            h: 200,
        });

        let shot = capture_estimate(chip(CaptureKind::Screenshot), selection, &displays);
        assert_eq!(
            shot.rate_bps, 0,
            "a screenshot has no recording rate, so the chip shows no time headroom"
        );

        let recording = capture_estimate(chip(CaptureKind::Recording), selection, &displays);
        assert!(
            recording.rate_bps > 0,
            "a recording scales its rate with the captured pixels"
        );

        let empty = capture_estimate(chip(CaptureKind::Recording), None, &displays);
        assert_eq!(
            empty.rate_bps, 0,
            "no selection yet means no recording estimate"
        );
    }

    #[test]
    fn shared_transients_target_the_physical_monitor_in_a_spanning_viewport() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(4480.0), px(1440.0)));
        let secondary = Bounds::new(point(px(2560.0), px(0.0)), size(px(1920.0), px(1080.0)));

        assert_eq!(
            ToastLayout::Status.placement().projected_bounds(
                secondary,
                ToastLayout::Status.size(),
                viewport,
            ),
            Some(Bounds::new(
                point(px(3260.0), px(48.0)),
                size(px(520.0), px(78.0)),
            )),
            "the guide belongs to the physical display, not the full desktop viewport"
        );
        assert_eq!(
            MonitorPlacement::top_center(CHIP_TOP).projected_bounds(
                secondary,
                size(px(CHIP_W), px(CHIP_H)),
                viewport,
            ),
            Some(Bounds::new(
                point(px(3370.0), px(CHIP_TOP)),
                size(px(CHIP_W), px(CHIP_H)),
            )),
            "the capture chip uses the same monitor-relative projection contract"
        );
    }

    #[test]
    fn format_bytes_uses_decimal_gb_mb_kb() {
        let cases = [
            (84_600_000_000, "84.6 GB"),
            (2_000_000_000, "2.0 GB"),
            (512_000_000, "512 MB"),
            (1_000_000, "1 MB"),
            (4_096, "4 KB"),
        ];
        for (bytes, expected) in cases {
            assert_eq!(format_bytes(bytes), expected, "bytes: {bytes}");
        }
    }

    #[test]
    fn format_duration_steps_through_sec_min_hours() {
        let cases = [
            (45, "45 sec"),
            (60, "1 min"),
            (540, "9 min"),
            (3_600, "1 h 0 min"),
            (15_135, "4 h 12 min"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(format_duration(seconds), expected, "seconds: {seconds}");
        }
    }

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
    fn resync_prefers_drag_then_pointer_then_noop() {
        let (tx, _rx) = mpsc::channel();
        let laptop = Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let external = Bounds::new(point(px(-1512.0), px(458.0)), size(px(1512.0), px(982.0)));
        let mut state = SelectionState::new(
            tx,
            Some(laptop),
            None,
            vec![laptop, external],
            Vec::new(),
            CaptureKind::Recording,
        );

        state.drag_current = Some(point(px(-800.0), px(700.0)));
        assert!(state.resync_active_bounds(Some(point(px(800.0), px(700.0))), None));
        assert_eq!(
            state.active_bounds,
            Some(external),
            "an in-progress drag wins over the live pointer"
        );

        state.drag_current = None;
        assert!(state.resync_active_bounds(Some(point(px(800.0), px(700.0))), None));
        assert_eq!(
            state.active_bounds,
            Some(laptop),
            "with no drag the live pointer drives the active monitor"
        );

        assert!(
            !state.resync_active_bounds(None, None),
            "no drag and no pointer leaves the active monitor untouched"
        );
        assert_eq!(state.active_bounds, Some(laptop));
    }

    #[test]
    fn resync_prefers_active_signal_when_not_dragging() {
        let (tx, _rx) = mpsc::channel();
        let laptop = Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let external = Bounds::new(point(px(-1512.0), px(458.0)), size(px(1512.0), px(982.0)));
        let mut state = SelectionState::new(
            tx,
            Some(laptop),
            None,
            vec![laptop, external],
            Vec::new(),
            CaptureKind::Recording,
        );

        assert!(state.resync_active_bounds(Some(point(px(800.0), px(700.0))), Some(external)));
        assert_eq!(
            state.active_bounds,
            Some(external),
            "the runtime active monitor signal places the message when no drag is in progress"
        );

        state.drag_current = Some(point(px(800.0), px(700.0)));
        assert!(state.resync_active_bounds(None, Some(external)));
        assert_eq!(
            state.active_bounds,
            Some(laptop),
            "an in-progress drag still anchors the message to the dragged monitor"
        );
    }

    #[test]
    fn gpui_drag_yields_to_the_active_global_pointer_poll() {
        let (tx, _rx) = mpsc::channel();
        let screen = Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let mut state = SelectionState::new(
            tx,
            Some(screen),
            None,
            vec![screen],
            Vec::new(),
            CaptureKind::Recording,
        );

        assert!(
            !state.apply_gpui_drag(point(px(10.0), px(10.0))),
            "no drag in progress means there is nothing for a move event to update"
        );

        state.drag_start = Some(point(px(100.0), px(100.0)));
        assert!(
            state.apply_gpui_drag(point(px(140.0), px(160.0))),
            "with no global-pointer poll the gpui move event drives the drag"
        );
        assert_eq!(state.drag_current, Some(point(px(140.0), px(160.0))));

        state.polling = true;
        assert!(
            !state.apply_gpui_drag(point(px(900.0), px(900.0))),
            "an active poll owns drag_current, so the gpui writer must yield"
        );
        assert_eq!(
            state.drag_current,
            Some(point(px(140.0), px(160.0))),
            "two writers fighting over drag_current is what flickered the selection between sizes"
        );
    }

    #[test]
    fn active_bounds_follow_drag_pointer_monitor() {
        let (tx, _rx) = mpsc::channel();
        let laptop = Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let external = Bounds::new(point(px(-1512.0), px(458.0)), size(px(1512.0), px(982.0)));
        let mut state = SelectionState::new(
            tx,
            Some(laptop),
            None,
            vec![laptop, external],
            Vec::new(),
            CaptureKind::Recording,
        );

        assert!(state.set_active_bounds_for_point(point(px(-800.0), px(700.0))));
        assert_eq!(state.active_bounds, Some(external));
        assert!(!state.set_active_bounds_for_point(point(px(-800.0), px(200.0))));
        assert_eq!(state.active_bounds, Some(external));
    }

    #[test]
    fn default_target_drives_selection_until_drag_has_area() {
        let (tx, _rx) = mpsc::channel();
        let screen = Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0)));
        let target = Rect {
            x: 100,
            y: 120,
            w: 800,
            h: 600,
        };
        let mut state = SelectionState::new(
            tx,
            Some(screen),
            Some(DetectedTarget::Window(target)),
            vec![screen],
            Vec::new(),
            CaptureKind::Recording,
        );

        assert_eq!(selection_global_rect(&state), Some(target));

        state.drag_start = Some(point(px(400.0), px(400.0)));
        state.drag_current = Some(point(px(400.0), px(400.0)));
        assert_eq!(
            selection_global_rect(&state),
            Some(target),
            "a click without area captures the detected target"
        );

        state.drag_current = Some(point(px(640.0), px(500.0)));
        assert_eq!(
            selection_global_rect(&state),
            Some(Rect {
                x: 400,
                y: 400,
                w: 240,
                h: 100,
            }),
            "a real drag overrides the detected target"
        );
    }

    #[test]
    fn default_target_bounds_are_clipped_to_selector_window() {
        let window = Bounds::new(point(px(100.0), px(100.0)), size(px(200.0), px(200.0)));
        let target = Rect {
            x: 50,
            y: 120,
            w: 120,
            h: 260,
        };

        assert_eq!(
            selection_bounds_in_window(target, window),
            Some(Bounds::new(
                point(px(0.0), px(20.0)),
                size(px(70.0), px(180.0))
            ))
        );
    }

    #[test]
    fn window_local_points_resolve_to_their_own_monitor() {
        let laptop = Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let external = Bounds::new(point(px(-1512.0), px(746.0)), size(px(1512.0), px(982.0)));
        let monitors = [laptop, external];
        let local = point(px(200.0), px(100.0));

        for owner in monitors {
            let global = point(owner.origin.x + local.x, owner.origin.y + local.y);
            assert_eq!(
                monitor_at_point(&monitors, global),
                Some(owner),
                "unified window origin keeps a window-local point on its own monitor (origin {:?})",
                owner.origin
            );
        }

        let per_display_origin = point(px(0.0), px(34.0));
        let per_display_point = point(
            per_display_origin.x + local.x,
            per_display_origin.y + local.y,
        );
        assert_eq!(
            monitor_at_point(&monitors, per_display_point),
            Some(laptop),
            "regression guard: per-display window.bounds() origin lands the external \
             window's point on the laptop, which flipped active_bounds every frame"
        );
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
