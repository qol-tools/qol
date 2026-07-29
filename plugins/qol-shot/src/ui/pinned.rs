use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use futures::channel::oneshot;
use gpui::*;

use crate::capture::actions::ShotAction;
use crate::capture::screenshot::CaptureFileReady;
use crate::platform;
use crate::ui::preview::{current_palette, PREVIEW_APP_ID};
use crate::ui::shortcuts::shot_action_for_keystroke;

const MIN_DIM: f32 = 48.0;
const MAX_DIM: f32 = 4096.0;
const EDGE: f32 = 8.0;
const CIRCLE: f32 = 36.0;
const CIRCLE_GAP: f32 = 10.0;
const CLOSE_CIRCLE: f32 = 26.0;
const SCROLL_STEP: f32 = 1.1;
const PIXELS_PER_NOTCH: f32 = 60.0;
const RESIZE_TICK: std::time::Duration = std::time::Duration::from_millis(8);
const SCROLL_COMMIT: std::time::Duration = std::time::Duration::from_millis(200);
const PIN_CACHE_CAPACITY: usize = 2;
const PIN_CACHE_SIZE: (f32, f32) = (360.0, 240.0);

static PIN_SEQ: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static PIN_CACHE: RefCell<VecDeque<WindowHandle<PinnedView>>> =
        const { RefCell::new(VecDeque::new()) };
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PinnedDismiss {
    Quit,
    Remove,
}

#[derive(Clone)]
pub struct PinnedContent {
    pub path: PathBuf,
    pub image: Option<Arc<RenderImage>>,
    pub size: (f32, f32),
    pub file_ready: CaptureFileReady,
    pub started_at: Instant,
}

struct PinnedWindowSpec {
    title: String,
    bounds: Bounds<Pixels>,
    dismiss: PinnedDismiss,
    border: bool,
    reveal: Option<PinReveal>,
    active: bool,
    focus: bool,
    cacheable: bool,
}

#[derive(Clone)]
struct PinReveal {
    origin: (f64, f64),
    source_preview: Option<String>,
}

type FullResolutionImage = anyhow::Result<(Arc<RenderImage>, u32, u32)>;

fn spawn_full_resolution_load(
    file_ready: CaptureFileReady,
    path: PathBuf,
) -> anyhow::Result<oneshot::Receiver<FullResolutionImage>> {
    let (sender, receiver) = oneshot::channel();
    let worker = std::thread::Builder::new()
        .name("qol-shot-pin-image".to_string())
        .spawn(move || {
            let result = file_ready
                .wait()
                .and_then(|()| crate::ui::preview::read_render_image(&path));
            let _ = sender.send(result);
        })
        .map_err(|error| anyhow::anyhow!("failed to start pinned image worker: {error}"))?;
    drop(worker);
    Ok(receiver)
}

pub fn pre_create(cx: &mut App) {
    if !platform::pin_cache_enabled() {
        return;
    }
    for _ in 0..PIN_CACHE_CAPACITY {
        let seq = PIN_SEQ.fetch_add(1, Ordering::Relaxed);
        let title = format!("qol-shot-pin-{}-{seq}", std::process::id());
        let origin = point(px(-100.0), px(-100.0));
        let bounds = Bounds {
            origin,
            size: size(px(PIN_CACHE_SIZE.0), px(PIN_CACHE_SIZE.1)),
        };
        let content = PinnedContent {
            path: PathBuf::new(),
            image: None,
            size: PIN_CACHE_SIZE,
            file_ready: CaptureFileReady::ready(),
            started_at: Instant::now(),
        };
        let spec = PinnedWindowSpec {
            title: title.clone(),
            bounds,
            dismiss: PinnedDismiss::Remove,
            border: false,
            reveal: None,
            active: false,
            focus: false,
            cacheable: true,
        };
        let Some(handle) = open_window(content, spec, cx) else {
            continue;
        };
        if !platform::prepare_pin_window(&title, (-100.0, -100.0)) {
            let _ = handle.update(cx, |_view, window, _cx| window.remove_window());
            continue;
        }
        PIN_CACHE.with(|cache| cache.borrow_mut().push_back(handle));
    }
    PIN_CACHE.with(|cache| {
        qol_runtime::probe!("SHOT_PIN_PRECREATE", "windows={}", cache.borrow().len());
    });
}

pub fn open(
    content: PinnedContent,
    origin: Point<Pixels>,
    dismiss: PinnedDismiss,
    source_preview: Option<String>,
    cx: &mut App,
) -> bool {
    let reveal = PinReveal {
        origin: (origin.x.to_f64(), origin.y.to_f64()),
        source_preview,
    };
    if platform::pin_cache_enabled()
        && cache::open(content.clone(), origin, dismiss, reveal.clone(), cx)
    {
        return true;
    }
    let seq = PIN_SEQ.fetch_add(1, Ordering::Relaxed);
    let title = format!("qol-shot-pin-{}-{seq}", std::process::id());
    let bounds = Bounds {
        origin,
        size: size(px(content.size.0), px(content.size.1)),
    };
    let config = crate::config::load();
    let border = config.capture.pin_border;
    let spec = PinnedWindowSpec {
        title: title.clone(),
        bounds,
        dismiss,
        border,
        reveal: Some(reveal),
        active: true,
        focus: true,
        cacheable: false,
    };
    let opened_at = Instant::now();
    let Some(_handle) = open_window(content, spec, cx) else {
        eprintln!("[qol-shot] pinned window open failed");
        return false;
    };
    platform::after_pin_open(&title);
    qol_runtime::probe!(
        "SHOT_PIN_OPEN",
        "path=create ms={}",
        opened_at.elapsed().as_millis()
    );
    true
}

fn open_window(
    content: PinnedContent,
    spec: PinnedWindowSpec,
    cx: &mut App,
) -> Option<WindowHandle<PinnedView>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(spec.bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: qol_gpui::popup_window::pinned_window_kind(),
        focus: spec.focus,
        show: true,
        is_movable: true,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(PREVIEW_APP_ID.to_string()),
        ..Default::default()
    };
    let focus = spec.focus;
    let handle = cx
        .open_window(options, move |window, cx| {
            window.set_window_title(&spec.title);
            let view = cx.new(|cx| PinnedView::new(content, spec, cx));
            if focus {
                window.focus(&view.focus_handle(cx));
                window.activate_window();
            }
            view
        })
        .ok()?;
    let _ = handle.update(cx, |view, _window, cx| {
        view.handle = Some(handle);
        view.start_full_resolution_upgrade(cx);
    });
    Some(handle)
}

mod cache {
    use super::*;

    pub fn open(
        content: PinnedContent,
        origin: Point<Pixels>,
        dismiss: PinnedDismiss,
        reveal: PinReveal,
        cx: &mut App,
    ) -> bool {
        let opened_at = Instant::now();
        let Some(handle) = PIN_CACHE.with(|cache| cache.borrow_mut().pop_front()) else {
            return false;
        };
        let config = crate::config::load();
        let border = config.capture.pin_border;
        let content_size = content.size;
        let title = handle
            .update(cx, |view, window, cx| {
                let title = view.title.clone();
                reset(view, content, dismiss, border, reveal, cx);
                view.start_full_resolution_upgrade(cx);
                qol_gpui::popup_window::sync_window_layout(
                    &title,
                    window,
                    origin,
                    size(px(content_size.0), px(content_size.1)),
                );
                window.focus(&view.focus_handle(cx));
                window.activate_window();
                title
            })
            .ok();
        if title.is_none() {
            return false;
        }
        qol_runtime::probe!(
            "SHOT_PIN_OPEN",
            "path=reuse ms={}",
            opened_at.elapsed().as_millis()
        );
        cx.activate(true);
        true
    }

    fn reset(
        view: &mut PinnedView,
        content: PinnedContent,
        dismiss: PinnedDismiss,
        border: bool,
        reveal: PinReveal,
        cx: &mut Context<PinnedView>,
    ) {
        view.path = content.path;
        view.image = content.image;
        view.file_ready = content.file_ready;
        view.dismiss = dismiss;
        view.border = border;
        view.hovered = false;
        view.ratio = if content.size.1 > 0.0 {
            content.size.0 / content.size.1
        } else {
            1.0
        };
        view.started_at = content.started_at;
        view.active = true;
        view.action_pending = false;
        view.first_paint = true;
        view.reveal_generation = view.reveal_generation.wrapping_add(1);
        view.scheduled_reveal_generation = None;
        view.pending_reveal = Some(reveal);
        cx.notify();
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct PinRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct ResizeDrag {
    edge: Option<ResizeEdge>,
    start: PinRect,
    pointer_start: (f32, f32),
    bounds: PinRect,
    canvas: Option<PinRect>,
    anchors: (bool, bool),
    session: platform::PinResizeSession,
}

#[derive(Clone, Copy)]
struct ScrollResize {
    commit_at: Instant,
}

pub struct PinnedView {
    path: PathBuf,
    image: Option<Arc<RenderImage>>,
    file_ready: CaptureFileReady,
    title: String,
    dismiss: PinnedDismiss,
    border: bool,
    hovered: bool,
    ratio: f32,
    resize_drag: Option<ResizeDrag>,
    scroll_resize: Option<ScrollResize>,
    scroll_remainder: f32,
    resize_loop_armed: bool,
    resize_loop_epoch: u64,
    started_at: Instant,
    active: bool,
    action_pending: bool,
    first_paint: bool,
    reveal_generation: u64,
    scheduled_reveal_generation: Option<u64>,
    pending_reveal: Option<PinReveal>,
    cacheable: bool,
    handle: Option<WindowHandle<PinnedView>>,
    focus_handle: FocusHandle,
}

impl PinnedView {
    fn new(content: PinnedContent, spec: PinnedWindowSpec, cx: &mut Context<Self>) -> Self {
        let ratio = if content.size.1 > 0.0 {
            content.size.0 / content.size.1
        } else {
            1.0
        };
        let reveal_generation = u64::from(spec.reveal.is_some());
        Self {
            path: content.path,
            image: content.image,
            file_ready: content.file_ready,
            title: spec.title,
            dismiss: spec.dismiss,
            border: spec.border,
            hovered: false,
            ratio,
            resize_drag: None,
            scroll_resize: None,
            scroll_remainder: 0.0,
            resize_loop_armed: false,
            resize_loop_epoch: 0,
            started_at: content.started_at,
            active: spec.active,
            action_pending: false,
            first_paint: true,
            reveal_generation,
            scheduled_reveal_generation: None,
            pending_reveal: spec.reveal,
            cacheable: spec.cacheable,
            handle: None,
            focus_handle: cx.focus_handle(),
        }
    }

    fn start_full_resolution_upgrade(&self, cx: &mut Context<Self>) {
        if self.image.is_none() || self.path.as_os_str().is_empty() {
            return;
        }
        let file_ready = self.file_ready.clone();
        let path = self.path.clone();
        let generation = self.reveal_generation;
        let started = Instant::now();
        let receiver = match spawn_full_resolution_load(file_ready, path) {
            Ok(receiver) => receiver,
            Err(error) => {
                eprintln!("[qol-shot] pinned image worker failed: {error:#}");
                return;
            }
        };
        qol_runtime::probe!("SHOT_PIN_IMAGE", "generation={generation} state=scheduled");
        cx.spawn(async move |view, cx| {
            let result = match receiver.await {
                Ok(result) => result,
                Err(error) => Err(anyhow::anyhow!("pinned image worker stopped: {error}")),
            };
            let _ = view.update(cx, move |view, cx| {
                view.finish_full_resolution_upgrade(generation, started, result, cx);
            });
        })
        .detach();
    }

    fn finish_full_resolution_upgrade(
        &mut self,
        generation: u64,
        started: Instant,
        result: FullResolutionImage,
        cx: &mut Context<Self>,
    ) {
        if !self.active || self.reveal_generation != generation {
            return;
        }
        let (image, width, height) = match result {
            Ok(image) => image,
            Err(error) => {
                qol_runtime::probe!("SHOT_PIN_IMAGE", "generation={generation} state=failed");
                eprintln!("[qol-shot] full-resolution pinned image failed: {error:#}");
                return;
            }
        };
        self.image = Some(image);
        qol_runtime::probe!(
            "SHOT_PIN_IMAGE",
            "generation={generation} state=upgraded dims={width}x{height} ms={}",
            started.elapsed().as_millis()
        );
        cx.notify();
    }

    fn schedule_reveal_after_present(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.reveal_generation;
        if self.pending_reveal.is_none() || self.scheduled_reveal_generation == Some(generation) {
            return;
        }
        self.scheduled_reveal_generation = Some(generation);
        qol_runtime::probe!(
            "SHOT_PIN_REVEAL",
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
                "SHOT_PIN_REVEAL",
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
            "SHOT_PIN_REVEAL",
            "title={} generation={generation} action_ms={} state=presented",
            self.title,
            self.started_at.elapsed().as_millis()
        );
        platform::configure_pin_window(self.title.clone(), reveal.origin, reveal.source_preview);
    }

    fn begin_drag(
        &mut self,
        edge: Option<ResizeEdge>,
        local: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        platform::pin_focus(&self.title);
        if self.scroll_resize.is_some() {
            self.end_resize(window, cx);
        }
        let Some(session) = platform::pin_resize_session(&self.title) else {
            match edge {
                Some(edge) => window.start_window_resize(edge),
                None => qol_gpui::platform::start_window_move(window),
            }
            return;
        };
        let start = session.bounds().map(|(x, y, w, h)| PinRect { x, y, w, h });
        let start = start.unwrap_or_else(|| {
            let bounds = window.bounds();
            PinRect {
                x: bounds.origin.x.to_f64() as f32,
                y: bounds.origin.y.to_f64() as f32,
                w: bounds.size.width.to_f64() as f32,
                h: bounds.size.height.to_f64() as f32,
            }
        });
        let anchors = edge.map(edge_anchors).unwrap_or((false, false));
        let canvas = edge.map(|_| {
            session.anchor(anchors.0, anchors.1);
            let canvas = drag_canvas(start, anchors);
            session.apply(canvas.x, canvas.y, canvas.w, canvas.h);
            canvas
        });
        let pointer_start = (
            start.x + local.x.to_f64() as f32,
            start.y + local.y.to_f64() as f32,
        );
        self.resize_drag = Some(ResizeDrag {
            edge,
            start,
            pointer_start,
            bounds: start,
            canvas,
            anchors,
            session,
        });
        self.spawn_resize_loop(window, cx);
    }

    fn spawn_resize_loop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.resize_loop_armed {
            return;
        }
        self.resize_loop_armed = true;
        let epoch = self.resize_loop_epoch;
        let pointer_session = self
            .scroll_resize
            .is_none()
            .then(|| self.resize_drag.as_ref().map(|drag| drag.session.clone()))
            .flatten();
        cx.spawn_in(window, async move |view, cx| loop {
            cx.background_executor().timer(RESIZE_TICK).await;
            let pointer = match pointer_session.clone() {
                Some(session) => {
                    cx.background_executor()
                        .spawn(async move { session.pointer() })
                        .await
                }
                None => None,
            };
            let live = view
                .update_in(cx, |view, window, cx| {
                    if view.resize_loop_epoch != epoch {
                        return false;
                    }
                    view.resize_tick(pointer, window, cx)
                })
                .unwrap_or(false);
            if !live {
                let _ = view.update_in(cx, |view, _, _| {
                    if view.resize_loop_epoch == epoch {
                        view.resize_loop_armed = false;
                    }
                });
                break;
            }
        })
        .detach();
    }

    fn resize_tick(
        &mut self,
        pointer: Option<(f32, f32)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.scroll_resize.is_some() {
            return self.scroll_resize_tick(window, cx);
        }
        let ratio = self.ratio;
        let Some(drag) = self.resize_drag.as_ref() else {
            return false;
        };
        let Some(pointer) = pointer else {
            return true;
        };
        let next = drag_bounds(drag.start, drag.edge, drag.pointer_start, pointer, ratio);
        self.apply_resize_bounds(next, cx);
        true
    }

    fn apply_resize_bounds(&mut self, next: PinRect, cx: &mut Context<Self>) {
        let Some(drag) = self.resize_drag.as_mut() else {
            return;
        };
        if next == drag.bounds {
            return;
        }
        drag.bounds = next;
        match drag.canvas {
            None => {
                drag.session.apply(next.x, next.y, next.w, next.h);
                qol_runtime::probe!(
                    "SHOT_PIN_TICK",
                    "mode=move pos=({:.0},{:.0})",
                    next.x,
                    next.y,
                );
            }
            Some(canvas) => {
                if !canvas_contains(canvas, next) {
                    let grown = drag_canvas(next, drag.anchors);
                    drag.session.apply(grown.x, grown.y, grown.w, grown.h);
                    drag.canvas = Some(grown);
                }
                cx.notify();
            }
        }
    }

    fn scroll_resize_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(scroll) = self.scroll_resize else {
            return false;
        };
        if Instant::now() < scroll.commit_at {
            return true;
        }
        self.end_resize(window, cx);
        false
    }

    fn canvas_drag_rects(&self) -> Option<(PinRect, PinRect)> {
        let drag = self.resize_drag.as_ref()?;
        drag.canvas.map(|canvas| (canvas, drag.bounds))
    }

    fn end_resize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scroll = self.scroll_resize.take().is_some();
        self.scroll_remainder = 0.0;
        self.resize_loop_armed = false;
        self.resize_loop_epoch = self.resize_loop_epoch.wrapping_add(1);
        let Some(drag) = self.resize_drag.take() else {
            return;
        };
        let last = if scroll {
            drag.bounds
        } else {
            drag.session
                .pointer()
                .map(|pointer| {
                    drag_bounds(
                        drag.start,
                        drag.edge,
                        drag.pointer_start,
                        pointer,
                        self.ratio,
                    )
                })
                .unwrap_or(drag.bounds)
        };
        if drag.canvas.is_none() {
            if last != drag.bounds {
                drag.session.apply(last.x, last.y, last.w, last.h);
            }
            qol_runtime::probe!(
                "SHOT_PIN_TICK",
                "mode=move stage=release pos=({:.0},{:.0}) corrected={}",
                last.x,
                last.y,
                last != drag.bounds,
            );
            return;
        }
        window.resize(size(px(last.w), px(last.h)));
        drag.session.move_to(last.x, last.y);
        qol_runtime::probe!(
            "SHOT_PIN_RESIZE",
            "stage=request mode={} requested=({:.0},{:.0}) {}x{}",
            if scroll { "scroll" } else { "pointer" },
            last.x,
            last.y,
            last.w,
            last.h,
        );
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        platform::pin_release_focus(&self.title);
        qol_gpui::popup_window::restore_composite(&self.title);
        if platform::pin_cache_enabled() && self.dismiss == PinnedDismiss::Remove && self.cacheable
        {
            self.recycle(window, cx);
            return;
        }
        match self.dismiss {
            PinnedDismiss::Quit => cx.quit(),
            PinnedDismiss::Remove => window.remove_window(),
        }
    }

    fn recycle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(handle) = self.handle else {
            window.remove_window();
            return;
        };
        self.active = false;
        self.image = None;
        self.path.clear();
        self.resize_drag = None;
        self.scroll_resize = None;
        self.resize_loop_armed = false;
        self.resize_loop_epoch = self.resize_loop_epoch.wrapping_add(1);
        let origin = point(px(-100.0), px(-100.0));
        qol_gpui::popup_window::sync_window_layout(
            &self.title,
            window,
            origin,
            size(px(PIN_CACHE_SIZE.0), px(PIN_CACHE_SIZE.1)),
        );
        if !platform::prepare_pin_window(&self.title, (-100.0, -100.0)) {
            window.remove_window();
            return;
        }
        let cached = PIN_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() >= PIN_CACHE_CAPACITY {
                return false;
            }
            cache.push_back(handle);
            true
        });
        if !cached {
            window.remove_window();
            return;
        }
        qol_runtime::probe!("SHOT_PIN_RECYCLE", "title={} result=ok", self.title);
        cx.notify();
    }

    fn perform(&mut self, action: ShotAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.action_pending {
            return;
        }
        let Some(handle) = window.window_handle().downcast::<PinnedView>() else {
            return;
        };
        self.action_pending = true;
        let file_ready = self.file_ready.clone();
        let path = self.path.clone();
        let perform = move || {
            action.perform(&path)?;
            platform::show_notification(action.done_message(), &path.display().to_string(), 1400);
            Ok(())
        };
        if self.dismiss == PinnedDismiss::Remove {
            if let Err(error) =
                crate::capture::actions::spawn_file_action("pinned", action, file_ready, perform)
            {
                eprintln!("[qol-shot] pinned action worker failed: {error:#}");
                self.action_pending = false;
                return;
            }
            self.action_pending = false;
            self.close(window, cx);
            return;
        }
        let action_task = cx.background_spawn(async move {
            crate::capture::actions::perform_when_file_ready("pinned", action, file_ready, perform)
        });
        let reveal_generation = self.reveal_generation;
        cx.spawn(async move |_view, cx| {
            let _result = action_task.await;
            let _ = handle.update(cx, move |view, window, cx| {
                if !view.active || view.reveal_generation != reveal_generation {
                    return;
                }
                view.action_pending = false;
                view.close(window, cx);
            });
        })
        .detach();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(action) = shot_action_for_keystroke(&event.keystroke, ShotAction::Copy) {
            self.perform(action, window, cx);
            return;
        }

        match event.keystroke.key.as_str() {
            "escape" | "esc" => self.close(window, cx),
            _ => {}
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let notches = match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => pixels.y.to_f64() as f32 / PIXELS_PER_NOTCH,
        };
        if notches == 0.0 {
            return;
        }

        if self.resize_drag.is_some() && self.scroll_resize.is_none() {
            return;
        }

        let steps = scroll_steps(&mut self.scroll_remainder, notches);
        if steps == 0 {
            return;
        }

        if self.resize_drag.is_none() && !self.begin_scroll_resize(window) {
            resize_window_by_scroll(window, steps);
            return;
        }

        self.apply_scroll_steps(steps, cx);
        self.spawn_resize_loop(window, cx);
    }

    fn begin_scroll_resize(&mut self, window: &mut Window) -> bool {
        let Some(session) = platform::pin_resize_session(&self.title) else {
            return false;
        };
        let start = session_rect(&session).unwrap_or_else(|| window_rect(window));
        let canvas = drag_canvas(start, (false, false));
        session.anchor(false, false);
        session.apply(canvas.x, canvas.y, canvas.w, canvas.h);
        self.resize_drag = Some(ResizeDrag {
            edge: None,
            start,
            pointer_start: (start.x, start.y),
            bounds: start,
            canvas: Some(canvas),
            anchors: (false, false),
            session,
        });
        self.scroll_resize = Some(ScrollResize {
            commit_at: Instant::now() + SCROLL_COMMIT,
        });
        true
    }

    fn apply_scroll_steps(&mut self, steps: i32, cx: &mut Context<Self>) {
        let Some(current) = self.resize_drag.as_ref().map(|drag| drag.bounds) else {
            return;
        };
        let Some(scroll) = self.scroll_resize.as_mut() else {
            return;
        };
        scroll.commit_at = Instant::now() + SCROLL_COMMIT;
        let factor = clamp_scale_factor(SCROLL_STEP.powi(steps), current.w, current.h);
        if (factor - 1.0).abs() < f32::EPSILON {
            return;
        }
        self.apply_resize_bounds(scale_rect(current, factor), cx);
    }

    fn picture(&self) -> Img {
        let picture = match &self.image {
            Some(render_image) => img(render_image.clone()),
            None => img(self.path.clone()),
        };
        picture.size_full().object_fit(ObjectFit::Fill)
    }

    fn action_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = current_palette();
        div()
            .absolute()
            .bottom(px(EDGE + 4.0))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .gap(px(CIRCLE_GAP))
            .children(
                ShotAction::PINNED
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, action)| {
                        div()
                            .id(("pin-action", index))
                            .w(px(CIRCLE))
                            .h(px(CIRCLE))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_2()
                            .border_color(rgb(palette.action_border))
                            .bg(rgb(palette.action_bg))
                            .text_color(rgb(palette.action_glyph))
                            .child(action.glyph())
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.perform(action, window, cx)
                            }))
                    }),
            )
    }

    fn close_circle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = current_palette();
        div()
            .id("pin-close")
            .absolute()
            .top(px(EDGE))
            .right(px(EDGE))
            .w(px(CLOSE_CIRCLE))
            .h(px(CLOSE_CIRCLE))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .border_2()
            .border_color(rgb(palette.action_border))
            .bg(rgb(palette.action_bg))
            .text_color(rgb(palette.action_glyph))
            .child("✕")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)))
    }

    fn resize_zone(&self, edge: ResizeEdge, cx: &mut Context<Self>) -> Div {
        let zone = div().absolute().cursor(resize_cursor(edge)).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                this.begin_drag(Some(edge), event.position, window, cx);
            }),
        );
        match edge {
            ResizeEdge::Top => zone.top_0().left(px(EDGE)).right(px(EDGE)).h(px(EDGE)),
            ResizeEdge::Bottom => zone.bottom_0().left(px(EDGE)).right(px(EDGE)).h(px(EDGE)),
            ResizeEdge::Left => zone.left_0().top(px(EDGE)).bottom(px(EDGE)).w(px(EDGE)),
            ResizeEdge::Right => zone.right_0().top(px(EDGE)).bottom(px(EDGE)).w(px(EDGE)),
            ResizeEdge::TopLeft => zone.top_0().left_0().w(px(EDGE)).h(px(EDGE)),
            ResizeEdge::TopRight => zone.top_0().right_0().w(px(EDGE)).h(px(EDGE)),
            ResizeEdge::BottomLeft => zone.bottom_0().left_0().w(px(EDGE)).h(px(EDGE)),
            ResizeEdge::BottomRight => zone.bottom_0().right_0().w(px(EDGE)).h(px(EDGE)),
        }
    }
}

const RESIZE_EDGES: [ResizeEdge; 8] = [
    ResizeEdge::Top,
    ResizeEdge::Bottom,
    ResizeEdge::Left,
    ResizeEdge::Right,
    ResizeEdge::TopLeft,
    ResizeEdge::TopRight,
    ResizeEdge::BottomLeft,
    ResizeEdge::BottomRight,
];

impl Focusable for PinnedView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PinnedView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.schedule_reveal_after_present(window, cx);
        if !self.active {
            return div().id("shot-pin").size_full();
        }
        if self.first_paint {
            self.first_paint = false;
            qol_runtime::probe!(
                "SHOT_PIN_RENDER",
                "action_ms={} title={}",
                self.started_at.elapsed().as_millis(),
                self.title
            );
        }
        let palette = current_palette();
        if let Some((canvas, image)) = self.canvas_drag_rects() {
            let mut picture_frame = div()
                .absolute()
                .left(px(image.x - canvas.x))
                .top(px(image.y - canvas.y))
                .w(px(image.w))
                .h(px(image.h))
                .bg(rgb(palette.window_bg))
                .child(self.picture());
            if self.border {
                picture_frame = picture_frame
                    .border_1()
                    .border_color(rgb(palette.thumb_border));
            }
            return div()
                .id("shot-pin")
                .track_focus(&self.focus_handle)
                .on_key_down(cx.listener(Self::on_key))
                .on_scroll_wheel(cx.listener(Self::on_scroll))
                .size_full()
                .relative()
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, window, cx| this.end_resize(window, cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, window, cx| this.end_resize(window, cx)),
                )
                .child(picture_frame);
        }
        let show_controls =
            self.resize_drag.is_some() || (self.hovered && window.is_window_hovered());
        let mut root = div()
            .id("shot-pin")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .relative()
            .bg(rgb(palette.window_bg))
            .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                if this.hovered != *hovered {
                    this.hovered = *hovered;
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.begin_drag(None, event.position, window, cx)
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, window, cx| this.end_resize(window, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, window, cx| this.end_resize(window, cx)),
            )
            .child(self.picture());

        if self.border {
            root = root.border_1().border_color(rgb(palette.thumb_border));
        }

        if show_controls {
            let viewport = window.viewport_size();
            if action_row_fits(
                viewport.width.to_f64() as f32,
                viewport.height.to_f64() as f32,
            ) {
                root = root.child(self.action_row(cx));
            }
            root = root.child(self.close_circle(cx));
            for edge in RESIZE_EDGES {
                root = root.child(self.resize_zone(edge, cx));
            }
        }

        root
    }
}

fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

const CANVAS_FACTOR: f32 = 3.0;

fn drag_canvas(image: PinRect, anchors: (bool, bool)) -> PinRect {
    let scale = CANVAS_FACTOR
        .min(MAX_DIM / image.w)
        .min(MAX_DIM / image.h)
        .max(1.0);
    let w = image.w * scale;
    let h = image.h * scale;
    let x = if anchors.0 {
        image.x + image.w - w
    } else {
        image.x
    };
    let y = if anchors.1 {
        image.y + image.h - h
    } else {
        image.y
    };
    PinRect { x, y, w, h }
}

fn canvas_contains(canvas: PinRect, image: PinRect) -> bool {
    image.x >= canvas.x - 0.5
        && image.y >= canvas.y - 0.5
        && image.x + image.w <= canvas.x + canvas.w + 0.5
        && image.y + image.h <= canvas.y + canvas.h + 0.5
}

fn edge_anchors(edge: ResizeEdge) -> (bool, bool) {
    match edge {
        ResizeEdge::Right | ResizeEdge::Bottom | ResizeEdge::BottomRight => (false, false),
        ResizeEdge::Left | ResizeEdge::BottomLeft => (true, false),
        ResizeEdge::Top | ResizeEdge::TopRight => (false, true),
        ResizeEdge::TopLeft => (true, true),
    }
}

fn scale_rect(rect: PinRect, factor: f32) -> PinRect {
    PinRect {
        x: rect.x,
        y: rect.y,
        w: rect.w * factor,
        h: rect.h * factor,
    }
}

fn scroll_steps(remainder: &mut f32, notches: f32) -> i32 {
    if !notches.is_finite() || notches == 0.0 {
        return 0;
    }
    if *remainder != 0.0 && remainder.signum() != notches.signum() {
        *remainder = 0.0;
    }
    *remainder += notches;
    let steps = remainder.trunc() as i32;
    if steps == 0 {
        return 0;
    }
    if !(-1..=1).contains(&steps) {
        *remainder = 0.0;
        return steps.signum();
    }
    *remainder -= steps as f32;
    steps
}

fn drag_bounds(
    start: PinRect,
    edge: Option<ResizeEdge>,
    pointer_start: (f32, f32),
    pointer: (f32, f32),
    ratio: f32,
) -> PinRect {
    let dx = pointer.0 - pointer_start.0;
    let dy = pointer.1 - pointer_start.1;
    let Some(edge) = edge else {
        return PinRect {
            x: start.x + dx,
            y: start.y + dy,
            w: start.w,
            h: start.h,
        };
    };
    resize_rect(start, edge, dx, dy, ratio)
}

fn session_rect(session: &platform::PinResizeSession) -> Option<PinRect> {
    session.bounds().map(|(x, y, w, h)| PinRect { x, y, w, h })
}

fn window_rect(window: &Window) -> PinRect {
    let bounds = window.bounds();
    PinRect {
        x: bounds.origin.x.to_f64() as f32,
        y: bounds.origin.y.to_f64() as f32,
        w: bounds.size.width.to_f64() as f32,
        h: bounds.size.height.to_f64() as f32,
    }
}

fn resize_window_by_scroll(window: &mut Window, steps: i32) {
    let current = window.viewport_size();
    let factor = clamp_scale_factor(
        SCROLL_STEP.powi(steps),
        current.width.to_f64() as f32,
        current.height.to_f64() as f32,
    );
    if (factor - 1.0).abs() < f32::EPSILON {
        return;
    }
    window.resize(size(current.width * factor, current.height * factor));
}

fn resize_rect(start: PinRect, edge: ResizeEdge, dx: f32, dy: f32, ratio: f32) -> PinRect {
    if start.w <= 0.0 || start.h <= 0.0 || ratio <= 0.0 {
        return start;
    }
    let grow_right = (start.w + dx) / start.w;
    let grow_left = (start.w - dx) / start.w;
    let grow_bottom = (start.h + dy) / start.h;
    let grow_top = (start.h - dy) / start.h;
    let scale = match edge {
        ResizeEdge::Right => grow_right,
        ResizeEdge::Left => grow_left,
        ResizeEdge::Bottom => grow_bottom,
        ResizeEdge::Top => grow_top,
        ResizeEdge::BottomRight => grow_right.max(grow_bottom),
        ResizeEdge::TopRight => grow_right.max(grow_top),
        ResizeEdge::BottomLeft => grow_left.max(grow_bottom),
        ResizeEdge::TopLeft => grow_left.max(grow_top),
    };
    let (anchor_right, anchor_bottom) = edge_anchors(edge);
    let scale = clamp_scale_factor(scale.max(0.0), start.w, start.h);
    let w = start.w * scale;
    let h = w / ratio;
    let x = if anchor_right {
        start.x + start.w - w
    } else {
        start.x
    };
    let y = if anchor_bottom {
        start.y + start.h - h
    } else {
        start.y
    };
    PinRect { x, y, w, h }
}

fn action_row_fits(width: f32, height: f32) -> bool {
    let count = ShotAction::PINNED.len() as f32;
    let needed_width = count * CIRCLE + (count - 1.0) * CIRCLE_GAP + 2.0 * EDGE;
    let needed_height = CIRCLE + CLOSE_CIRCLE + 3.0 * EDGE;
    width >= needed_width && height >= needed_height
}

fn clamp_scale_factor(factor: f32, width: f32, height: f32) -> f32 {
    if width <= 0.0 || height <= 0.0 {
        return 1.0;
    }
    let min_factor = (MIN_DIM / width.min(height)).min(1.0);
    let max_factor = (MAX_DIM / width.max(height)).max(1.0);
    factor.clamp(min_factor, max_factor)
}

#[cfg(test)]
mod tests {
    use super::{
        action_row_fits, clamp_scale_factor, drag_bounds, resize_rect, scroll_steps, PinRect,
    };
    use gpui::ResizeEdge;

    #[test]
    fn resize_rect_locks_ratio_and_anchors_opposite_side() {
        let start = PinRect {
            x: 100.0,
            y: 50.0,
            w: 400.0,
            h: 200.0,
        };
        let cases = [
            (
                ResizeEdge::Right,
                100.0,
                0.0,
                PinRect {
                    x: 100.0,
                    y: 50.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
            (
                ResizeEdge::Bottom,
                0.0,
                50.0,
                PinRect {
                    x: 100.0,
                    y: 50.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
            (
                ResizeEdge::Left,
                -100.0,
                0.0,
                PinRect {
                    x: 0.0,
                    y: 50.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
            (
                ResizeEdge::Top,
                0.0,
                -50.0,
                PinRect {
                    x: 100.0,
                    y: 0.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
            (
                ResizeEdge::BottomRight,
                100.0,
                10.0,
                PinRect {
                    x: 100.0,
                    y: 50.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
            (
                ResizeEdge::TopLeft,
                -100.0,
                -10.0,
                PinRect {
                    x: 0.0,
                    y: 0.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
        ];
        for (edge, dx, dy, expected) in cases {
            let got = resize_rect(start, edge, dx, dy, 2.0);
            assert!(
                (got.x - expected.x).abs() < 0.01
                    && (got.y - expected.y).abs() < 0.01
                    && (got.w - expected.w).abs() < 0.01
                    && (got.h - expected.h).abs() < 0.01,
                "edge {edge:?} dx={dx} dy={dy} got {got:?} want {expected:?}"
            );
        }
    }

    #[test]
    fn resize_rect_clamps_to_min_dimension() {
        let start = PinRect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 100.0,
        };
        let got = resize_rect(start, ResizeEdge::Right, -300.0, 0.0, 2.0);
        assert!(
            (got.h - 48.0).abs() < 0.01 && (got.w - 96.0).abs() < 0.01,
            "got {got:?}"
        );
    }

    #[test]
    fn drag_bounds_uses_the_latest_root_pointer() {
        let start = PinRect {
            x: 100.0,
            y: 50.0,
            w: 400.0,
            h: 200.0,
        };
        let pointer_start = (500.0, 250.0);
        let cases = [
            (
                Some(ResizeEdge::Right),
                (700.0, 250.0),
                PinRect {
                    x: 100.0,
                    y: 50.0,
                    w: 600.0,
                    h: 300.0,
                },
            ),
            (
                Some(ResizeEdge::Left),
                (300.0, 250.0),
                PinRect {
                    x: -100.0,
                    y: 50.0,
                    w: 600.0,
                    h: 300.0,
                },
            ),
            (
                None,
                (650.0, 300.0),
                PinRect {
                    x: 250.0,
                    y: 100.0,
                    w: 400.0,
                    h: 200.0,
                },
            ),
        ];
        for (edge, pointer, expected) in cases {
            let got = drag_bounds(start, edge, pointer_start, pointer, 2.0);
            assert_eq!(got, expected, "edge={edge:?} pointer={pointer:?}");
        }
    }

    #[test]
    fn scroll_steps_emit_at_most_one_increment_per_event() {
        let mut remainder = 0.0;
        let cases = [
            (0.6, 0, 0.6),
            (0.6, 1, 0.2),
            (-0.6, 0, -0.6),
            (-0.6, -1, -0.2),
            (3.4, 1, 0.0),
            (-3.4, -1, 0.0),
            (0.0, 0, 0.0),
            (f32::NAN, 0, 0.0),
        ];
        for (notches, expected_steps, expected_remainder) in cases {
            assert_eq!(scroll_steps(&mut remainder, notches), expected_steps);
            assert!((remainder - expected_remainder).abs() < 0.001);
        }
    }

    #[test]
    fn action_row_fits_requires_room_for_circles() {
        let cases = [
            (400.0, 300.0, true),
            (98.0, 86.0, true),
            (97.0, 86.0, false),
            (98.0, 85.0, false),
            (48.0, 48.0, false),
        ];
        for (width, height, expected) in cases {
            assert_eq!(
                action_row_fits(width, height),
                expected,
                "size: {width}x{height}"
            );
        }
    }

    #[test]
    fn clamp_scale_factor_bounds_growth_and_shrink() {
        let cases = [
            (1.1, 400.0, 300.0, 1.1),
            (0.9, 400.0, 300.0, 0.9),
            (0.5, 60.0, 90.0, 0.8),
            (10.0, 3000.0, 2000.0, 4096.0 / 3000.0),
            (1.1, 0.0, 100.0, 1.0),
        ];
        for (factor, width, height, expected) in cases {
            let clamped = clamp_scale_factor(factor, width, height);
            assert!(
                (clamped - expected).abs() < 0.001,
                "factor={factor} size={width}x{height} got={clamped} want={expected}"
            );
        }
    }
}
