use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use gpui::*;

use crate::popup_window::WindowGeometrySession;

mod geometry;

use self::geometry::{resized_rect, NativeRect, ResizeLimits};

const EDGE_BAND: f32 = 5.0;
const CORNER_BAND: f32 = 10.0;
const RESIZE_TICK: Duration = Duration::from_millis(8);
const USER_SIZE_SETTLE: Duration = Duration::from_millis(250);

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

#[derive(Clone)]
pub struct ResizeFrame {
    state: Rc<FrameState>,
}

struct FrameState {
    title: Box<dyn Fn() -> String>,
    min_size: Cell<Size<Pixels>>,
    sizing: Arc<Mutex<Sizing>>,
    drag: RefCell<Option<Drag>>,
    epoch: Arc<AtomicU64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sizing {
    Idle,
    Dragging,
    Settling(Instant),
}

impl Sizing {
    fn counts_as_user(self, now: Instant) -> bool {
        match self {
            Self::Idle => false,
            Self::Dragging => true,
            Self::Settling(since) => now.duration_since(since) < USER_SIZE_SETTLE,
        }
    }
}

#[derive(Clone)]
struct Drag {
    session: WindowGeometrySession,
    edge: ResizeEdge,
    start: NativeRect,
    pointer_start: (i32, i32),
    limits: ResizeLimits,
    epoch: u64,
    sizing: Arc<Mutex<Sizing>>,
}

impl Drag {
    fn bounds_now(&self) -> Option<NativeRect> {
        let (x, y) = self.session.pointer_root()?;
        Some(resized_rect(
            self.start,
            self.edge,
            f64::from(x - self.pointer_start.0),
            f64::from(y - self.pointer_start.1),
            self.limits,
        ))
    }

    fn apply(&self, rect: NativeRect) {
        self.session.set_bounds(
            rect.x.round() as i32,
            rect.y.round() as i32,
            rect.width.round().max(1.0) as u32,
            rect.height.round().max(1.0) as u32,
        );
    }

    fn released(&self) -> bool {
        self.session
            .pointer_on()
            .is_some_and(|pointer| !pointer.pressed)
    }

    fn finish(&self, epochs: &AtomicU64) -> bool {
        let mut sizing = lock(&self.sizing);
        let finished = epochs
            .compare_exchange(
                self.epoch,
                self.epoch.wrapping_add(1),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok();
        if finished {
            *sizing = Sizing::Settling(Instant::now());
        }
        finished
    }
}

impl ResizeFrame {
    pub fn new(title: impl Fn() -> String + 'static, min_size: Size<Pixels>) -> Self {
        Self {
            state: Rc::new(FrameState {
                title: Box::new(title),
                min_size: Cell::new(min_size),
                sizing: Arc::new(Mutex::new(Sizing::Idle)),
                drag: RefCell::new(None),
                epoch: Arc::new(AtomicU64::new(0)),
            }),
        }
    }

    pub fn min_size(&self) -> Size<Pixels> {
        self.state.min_size.get()
    }

    pub fn set_min_size(&self, min_size: Size<Pixels>) {
        self.state.min_size.set(min_size);
    }

    pub fn is_user_sizing(&self) -> bool {
        lock(&self.state.sizing).counts_as_user(Instant::now())
    }

    pub fn end_user_sizing(&self) {
        *lock(&self.state.sizing) = Sizing::Idle;
    }

    pub fn zones(&self) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .children(RESIZE_EDGES.map(|edge| self.zone(edge)))
    }

    fn zone(&self, edge: ResizeEdge) -> Div {
        let pressed = self.clone();
        let released = self.clone();
        let released_outside = self.clone();
        let zone = div()
            .absolute()
            .cursor(resize_cursor(edge))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                pressed.begin(edge, window, cx);
            })
            .on_mouse_up(MouseButton::Left, move |_, _, _| released.end())
            .on_mouse_up_out(MouseButton::Left, move |_, _, _| released_outside.end());
        placed(zone, edge)
    }

    fn begin(&self, edge: ResizeEdge, window: &mut Window, cx: &mut App) {
        let epoch = {
            let mut sizing = lock(&self.state.sizing);
            *sizing = Sizing::Dragging;
            self.state
                .epoch
                .fetch_add(1, Ordering::SeqCst)
                .wrapping_add(1)
        };
        let Some(drag) = self.client_drag(edge, epoch, window, cx) else {
            self.state.drag.borrow_mut().take();
            window.start_window_resize(edge);
            return;
        };
        *self.state.drag.borrow_mut() = Some(drag.clone());
        spawn_resize_loop(drag, self.state.epoch.clone(), cx);
    }

    fn end(&self) {
        let Some(drag) = self.state.drag.borrow_mut().take() else {
            let mut sizing = lock(&self.state.sizing);
            if *sizing == Sizing::Dragging {
                *sizing = Sizing::Settling(Instant::now());
            }
            return;
        };
        if !drag.finish(&self.state.epoch) {
            return;
        }
        if let Some(last) = drag.bounds_now() {
            drag.apply(last);
        }
    }

    fn client_drag(&self, edge: ResizeEdge, epoch: u64, window: &Window, cx: &App) -> Option<Drag> {
        let session = crate::popup_window::window_geometry_session(&(self.state.title)())?;
        let (x, y, width, height) = session.bounds()?;
        let pointer_start = session.pointer_root()?;
        let scale = f64::from(window.scale_factor());
        let min_size = self.state.min_size.get();
        Some(Drag {
            edge,
            start: NativeRect {
                x: f64::from(x),
                y: f64::from(y),
                width: f64::from(width),
                height: f64::from(height),
            },
            pointer_start,
            limits: ResizeLimits {
                min_width: min_size.width.to_f64() * scale,
                min_height: min_size.height.to_f64() * scale,
                area: native_work_area(window, cx),
            },
            epoch,
            sizing: self.state.sizing.clone(),
            session,
        })
    }
}

fn spawn_resize_loop(drag: Drag, epochs: Arc<AtomicU64>, cx: &mut App) {
    let executor = cx.background_executor().clone();
    cx.background_spawn(async move {
        let mut applied = drag.start;
        loop {
            executor.timer(RESIZE_TICK).await;
            if epochs.load(Ordering::SeqCst) != drag.epoch {
                return;
            }
            let released = drag.released();
            if let Some(next) = drag.bounds_now().filter(|next| *next != applied) {
                drag.apply(next);
                applied = next;
            }
            if released {
                drag.finish(&epochs);
                return;
            }
        }
    })
    .detach();
}

fn lock(sizing: &Mutex<Sizing>) -> std::sync::MutexGuard<'_, Sizing> {
    sizing.lock().unwrap_or_else(PoisonError::into_inner)
}

fn native_work_area(window: &Window, cx: &App) -> Option<NativeRect> {
    let display = window.display(cx)?;
    let monitor = crate::monitor::native_display_bounds(display.as_ref());
    let area = crate::popup_window::work_area_within(monitor).unwrap_or(monitor);
    Some(NativeRect::of_bounds(area))
}

fn placed(zone: Div, edge: ResizeEdge) -> Div {
    match edge {
        ResizeEdge::Top => zone
            .top_0()
            .left(px(CORNER_BAND))
            .right(px(CORNER_BAND))
            .h(px(EDGE_BAND)),
        ResizeEdge::Bottom => zone
            .bottom_0()
            .left(px(CORNER_BAND))
            .right(px(CORNER_BAND))
            .h(px(EDGE_BAND)),
        ResizeEdge::Left => zone
            .left_0()
            .top(px(CORNER_BAND))
            .bottom(px(CORNER_BAND))
            .w(px(EDGE_BAND)),
        ResizeEdge::Right => zone
            .right_0()
            .top(px(CORNER_BAND))
            .bottom(px(CORNER_BAND))
            .w(px(EDGE_BAND)),
        ResizeEdge::TopLeft => zone.top_0().left_0().w(px(CORNER_BAND)).h(px(CORNER_BAND)),
        ResizeEdge::TopRight => zone.top_0().right_0().w(px(CORNER_BAND)).h(px(CORNER_BAND)),
        ResizeEdge::BottomLeft => zone
            .bottom_0()
            .left_0()
            .w(px(CORNER_BAND))
            .h(px(CORNER_BAND)),
        ResizeEdge::BottomRight => zone
            .bottom_0()
            .right_0()
            .w(px(CORNER_BAND))
            .h(px(CORNER_BAND)),
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

#[cfg(test)]
mod tests;
