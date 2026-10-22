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
const SETTLE_WAIT: Duration = Duration::from_millis(300);
const REAPPLY_AFTER: Duration = Duration::from_millis(50);
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
    release_seen: Rc<Cell<bool>>,
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
        let (x, y, width, height) = rect.rounded();
        self.session.set_bounds(x, y, width, height);
    }

    fn released(&self) -> bool {
        self.release_seen.get()
            || self
                .session
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
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                cx.stop_propagation();
                pressed.begin(edge, event.position, window, cx);
            })
            .on_mouse_up(MouseButton::Left, move |_, _, _| released.end())
            .on_mouse_up_out(MouseButton::Left, move |_, _, _| released_outside.end());
        placed(zone, edge)
    }

    fn begin(&self, edge: ResizeEdge, press: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let epoch = {
            let mut sizing = lock(&self.state.sizing);
            *sizing = Sizing::Dragging;
            self.state
                .epoch
                .fetch_add(1, Ordering::SeqCst)
                .wrapping_add(1)
        };
        let Some(drag) = self.client_drag(edge, press, epoch, window, cx) else {
            self.state.drag.borrow_mut().take();
            window.start_window_resize(edge);
            return;
        };
        *self.state.drag.borrow_mut() = Some(drag.clone());
        qol_runtime::probe!(
            "GPUI_RESIZE",
            "phase=begin epoch={} edge={edge:?} start={:?} pointer={:?}",
            drag.epoch,
            drag.start.rounded(),
            drag.pointer_start
        );
        if crate::platform::RESIZE_BY_WINDOW_MANAGER {
            window.start_window_resize(edge);
            watch_release(drag, self.state.epoch.clone(), Instant::now(), window);
            return;
        }
        let applied = drag.start;
        follow_pointer(drag, self.state.epoch.clone(), applied, window);
    }

    fn end(&self) {
        let Some(drag) = self.state.drag.borrow_mut().take() else {
            let mut sizing = lock(&self.state.sizing);
            if *sizing == Sizing::Dragging {
                *sizing = Sizing::Settling(Instant::now());
            }
            return;
        };
        drag.release_seen.set(true);
    }

    fn client_drag(
        &self,
        edge: ResizeEdge,
        press: Point<Pixels>,
        epoch: u64,
        window: &Window,
        cx: &App,
    ) -> Option<Drag> {
        let session = crate::popup_window::window_geometry_session(&(self.state.title)())?;
        let (x, y, width, height) = session.bounds()?;
        session.pointer_root()?;
        let scale = f64::from(window.scale_factor());
        let pointer_start = (
            x + (press.x.to_f64() * scale).round() as i32,
            y + (press.y.to_f64() * scale).round() as i32,
        );
        let min_size = self.state.min_size.get();
        let area = native_work_area(window, cx);
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
                min_width: capped(min_size.width.to_f64() * scale, area.map(|area| area.width)),
                min_height: capped(
                    min_size.height.to_f64() * scale,
                    area.map(|area| area.height),
                ),
                area,
            },
            epoch,
            sizing: self.state.sizing.clone(),
            release_seen: Rc::new(Cell::new(false)),
            session,
        })
    }
}

fn follow_pointer(drag: Drag, epochs: Arc<AtomicU64>, applied: NativeRect, window: &Window) {
    window.on_next_frame(move |window, cx| {
        if epochs.load(Ordering::SeqCst) != drag.epoch {
            return;
        }
        let released = drag.released();
        let drawn = drawn_size(window);
        let applied = match drag.bounds_now() {
            Some(next) if next != applied => {
                apply_after_present(drag.clone(), next, cx);
                next
            }
            _ => applied,
        };
        qol_runtime::probe!(
            "GPUI_RESIZE",
            "phase=step epoch={} drawn={drawn:?} applied={:?} released={released}",
            drag.epoch,
            applied.rounded()
        );
        if released {
            settle_at(
                drag,
                epochs,
                applied,
                Instant::now(),
                Instant::now(),
                window,
            );
            return;
        }
        follow_pointer(drag, epochs, applied, window);
    });
}

fn watch_release(drag: Drag, epochs: Arc<AtomicU64>, last_frame: Instant, window: &Window) {
    window.on_next_frame(move |window, _| {
        if epochs.load(Ordering::SeqCst) != drag.epoch {
            return;
        }
        let released = drag.released();
        qol_runtime::probe!(
            "GPUI_RESIZE",
            "phase=wm-frame epoch={} drawn={:?} frame_ms={} released={released}",
            drag.epoch,
            drawn_size(window),
            last_frame.elapsed().as_millis()
        );
        if released {
            drag.finish(&epochs);
            return;
        }
        watch_release(drag, epochs, Instant::now(), window);
    });
}

fn settle_at(
    drag: Drag,
    epochs: Arc<AtomicU64>,
    target: NativeRect,
    since: Instant,
    applied_at: Instant,
    window: &Window,
) {
    window.on_next_frame(move |window, cx| {
        if epochs.load(Ordering::SeqCst) != drag.epoch {
            return;
        }
        let actual = drag.session.bounds();
        if actual == Some(target.rounded()) || since.elapsed() > SETTLE_WAIT {
            qol_runtime::probe!(
                "GPUI_RESIZE",
                "phase=settled epoch={} target={:?} actual={actual:?} waited_ms={}",
                drag.epoch,
                target.rounded(),
                since.elapsed().as_millis()
            );
            drag.finish(&epochs);
            return;
        }
        let applied_at = if applied_at.elapsed() > REAPPLY_AFTER {
            apply_after_present(drag.clone(), target, cx);
            Instant::now()
        } else {
            applied_at
        };
        settle_at(drag, epochs, target, since, applied_at, window);
    });
}

fn drawn_size(window: &Window) -> (i32, i32) {
    let scale = window.scale_factor();
    let size = window.bounds().size;
    (
        (size.width.to_f64() * f64::from(scale)).round() as i32,
        (size.height.to_f64() * f64::from(scale)).round() as i32,
    )
}

fn apply_after_present(drag: Drag, rect: NativeRect, cx: &mut App) {
    cx.spawn(async move |_| drag.apply(rect)).detach();
}

fn lock(sizing: &Mutex<Sizing>) -> std::sync::MutexGuard<'_, Sizing> {
    sizing.lock().unwrap_or_else(PoisonError::into_inner)
}

fn native_work_area(window: &Window, cx: &App) -> Option<NativeRect> {
    let display = window.display(cx)?;
    let monitor = crate::monitor::native_display_bounds(display.as_ref());
    let area = crate::popup_window::work_area_within(monitor).unwrap_or(monitor);
    Some(NativeRect::of_bounds(crate::monitor::windowed_area(
        monitor, area,
    )))
}

fn capped(min_span: f64, room: Option<f64>) -> f64 {
    room.map_or(min_span, |room| min_span.min(room))
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
