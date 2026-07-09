use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use gpui::*;

use crate::actions::ShotAction;
use crate::platform;
use crate::preview::{current_palette, PREVIEW_APP_ID};

const MIN_DIM: f32 = 48.0;
const MAX_DIM: f32 = 4096.0;
const EDGE: f32 = 8.0;
const CIRCLE: f32 = 36.0;
const CIRCLE_GAP: f32 = 10.0;
const CLOSE_CIRCLE: f32 = 26.0;
const SCROLL_STEP: f32 = 1.1;
const PIXELS_PER_NOTCH: f32 = 60.0;
const RESIZE_TICK: std::time::Duration = std::time::Duration::from_millis(8);
const REVEAL_TICK: std::time::Duration = std::time::Duration::from_millis(16);
const REVEAL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);
const SCROLL_COMMIT: std::time::Duration = std::time::Duration::from_millis(200);

static PIN_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PinnedDismiss {
    Quit,
    Remove,
}

pub struct PinnedContent {
    pub path: PathBuf,
    pub image: Option<Arc<RenderImage>>,
    pub size: (f32, f32),
}

pub fn open(
    content: PinnedContent,
    origin: Point<Pixels>,
    dismiss: PinnedDismiss,
    cx: &mut App,
) -> bool {
    let seq = PIN_SEQ.fetch_add(1, Ordering::Relaxed);
    let title = format!("qol-shot-pin-{}-{seq}", std::process::id());
    let bounds = Bounds {
        origin,
        size: size(px(content.size.0), px(content.size.1)),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::Normal,
        focus: true,
        is_movable: true,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(PREVIEW_APP_ID.to_string()),
        ..Default::default()
    };
    let border = crate::config::load().capture.pin_border;
    let placed = Arc::new(AtomicBool::new(false));
    let window_title = title.clone();
    let view_placed = placed.clone();
    let opened = cx.open_window(options, move |window, cx| {
        window.set_window_title(&window_title);
        let view =
            cx.new(|cx| PinnedView::new(content, window_title, dismiss, border, view_placed, cx));
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view
    });
    if opened.is_err() {
        eprintln!("[qol-shot] pinned window open failed");
        return false;
    }
    platform::configure_pin_window(title, (origin.x.to_f64(), origin.y.to_f64()), placed);
    true
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
    title: String,
    dismiss: PinnedDismiss,
    border: bool,
    hovered: bool,
    ratio: f32,
    resize_drag: Option<ResizeDrag>,
    scroll_resize: Option<ScrollResize>,
    scroll_remainder: f32,
    resize_loop_armed: bool,
    placed: Arc<AtomicBool>,
    reveal_deadline: Instant,
    focus_handle: FocusHandle,
}

impl PinnedView {
    fn new(
        content: PinnedContent,
        title: String,
        dismiss: PinnedDismiss,
        border: bool,
        placed: Arc<AtomicBool>,
        cx: &mut Context<Self>,
    ) -> Self {
        let ratio = if content.size.1 > 0.0 {
            content.size.0 / content.size.1
        } else {
            1.0
        };
        let view = Self {
            path: content.path,
            image: content.image,
            title,
            dismiss,
            border,
            hovered: false,
            ratio,
            resize_drag: None,
            scroll_resize: None,
            scroll_remainder: 0.0,
            resize_loop_armed: false,
            placed,
            reveal_deadline: Instant::now() + REVEAL_TIMEOUT,
            focus_handle: cx.focus_handle(),
        };
        view.spawn_reveal_poll(cx);
        view
    }

    fn revealed(&self) -> bool {
        self.placed.load(Ordering::Relaxed) || Instant::now() >= self.reveal_deadline
    }

    fn spawn_reveal_poll(&self, cx: &mut Context<Self>) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                loop {
                    async_cx.background_executor().timer(REVEAL_TICK).await;
                    let revealed = this.update(&mut async_cx, |view, cx| {
                        let revealed = view.revealed();
                        if revealed {
                            qol_runtime::probe!(
                                "SHOT_PIN_REVEAL",
                                "placed={} title={}",
                                view.placed.load(Ordering::Relaxed),
                                view.title,
                            );
                            cx.notify();
                        }
                        revealed
                    });
                    if !matches!(revealed, Ok(false)) {
                        break;
                    }
                }
            }
        })
        .detach();
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
            self.end_resize(cx);
        }
        let Some(session) = platform::pin_resize_session(&self.title) else {
            match edge {
                Some(edge) => window.start_window_resize(edge),
                None => window.start_window_move(),
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
        let pointer_start = session.pointer().unwrap_or((
            start.x + local.x.to_f64() as f32,
            start.y + local.y.to_f64() as f32,
        ));
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
        cx.spawn_in(window, async move |view, cx| loop {
            cx.background_executor().timer(RESIZE_TICK).await;
            let live = view
                .update_in(cx, |view, _window, cx| view.resize_tick(cx))
                .unwrap_or(false);
            if !live {
                break;
            }
        })
        .detach();
    }

    fn resize_tick(&mut self, cx: &mut Context<Self>) -> bool {
        if self.scroll_resize.is_some() {
            return self.scroll_resize_tick(cx);
        }
        let ratio = self.ratio;
        let Some(drag) = self.resize_drag.as_ref() else {
            self.resize_loop_armed = false;
            return false;
        };
        let Some(pointer) = drag.session.pointer() else {
            return true;
        };
        let dx = pointer.0 - drag.pointer_start.0;
        let dy = pointer.1 - drag.pointer_start.1;
        let next = match drag.edge {
            Some(edge) => resize_rect(drag.start, edge, dx, dy, ratio),
            None => PinRect {
                x: drag.start.x + dx,
                y: drag.start.y + dy,
                w: drag.start.w,
                h: drag.start.h,
            },
        };
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
            None => drag.session.apply(next.x, next.y, next.w, next.h),
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

    fn scroll_resize_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(scroll) = self.scroll_resize else {
            self.resize_loop_armed = false;
            return false;
        };
        if Instant::now() < scroll.commit_at {
            return true;
        }
        self.end_resize(cx);
        false
    }

    fn canvas_drag_rects(&self) -> Option<(PinRect, PinRect)> {
        let drag = self.resize_drag.as_ref()?;
        drag.canvas.map(|canvas| (canvas, drag.bounds))
    }

    fn end_resize(&mut self, cx: &mut Context<Self>) {
        self.scroll_resize = None;
        self.scroll_remainder = 0.0;
        self.resize_loop_armed = false;
        let Some(drag) = self.resize_drag.take() else {
            return;
        };
        if drag.canvas.is_some() {
            let last = drag.bounds;
            drag.session.apply(last.x, last.y, last.w, last.h);
            cx.notify();
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        platform::pin_release_focus(&self.title);
        qol_gpui::popup_window::restore_composite();
        match self.dismiss {
            PinnedDismiss::Quit => cx.quit(),
            PinnedDismiss::Remove => window.remove_window(),
        }
    }

    fn perform(&mut self, action: ShotAction, window: &mut Window, cx: &mut Context<Self>) {
        match action.perform(&self.path) {
            Ok(()) => platform::show_notification(
                action.done_message(),
                &self.path.display().to_string(),
                1400,
            ),
            Err(error) => eprintln!("[qol-shot] pinned action failed: {error:#}"),
        }
        self.close(window, cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" | "esc" => self.close(window, cx),
            other => {
                let accel = other.chars().next();
                if let Some(action) = ShotAction::ALL
                    .iter()
                    .copied()
                    .find(|a| Some(a.accel()) == accel)
                {
                    self.perform(action, window, cx);
                }
            }
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

        let steps = self.scroll_steps(notches);
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

    fn scroll_steps(&mut self, notches: f32) -> i32 {
        if !notches.is_finite() || notches == 0.0 {
            return 0;
        }
        if self.scroll_remainder != 0.0 && self.scroll_remainder.signum() != notches.signum() {
            self.scroll_remainder = 0.0;
        }
        self.scroll_remainder += notches;
        let steps = self.scroll_remainder.trunc() as i32;
        self.scroll_remainder -= steps as f32;
        steps
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
                ShotAction::ALL
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
        if !self.revealed() {
            return div().id("shot-pin").size_full();
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
                    cx.listener(|this, _: &MouseUpEvent, _window, cx| this.end_resize(cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _window, cx| this.end_resize(cx)),
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
                cx.listener(|this, _: &MouseUpEvent, _window, cx| this.end_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _window, cx| this.end_resize(cx)),
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
    let count = ShotAction::ALL.len() as f32;
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
    use super::{action_row_fits, clamp_scale_factor, resize_rect, PinRect};
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
