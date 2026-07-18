use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, Result};
use gpui::*;

use crate::monitor::MonitorTracker;

pub const CORNER_MARGIN: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    CornerStack(Corner),
    MonitorCenter,
}

pub enum SurfaceKind {
    Toast,
    Panel,
}

pub struct Surface {
    kind: SurfaceKind,
    title: String,
    anchor: Anchor,
    timeout: Option<Duration>,
    size: Size<Pixels>,
}

type CloseWindow = Box<dyn Fn(&mut App)>;

struct DismissState {
    close: RefCell<Option<CloseWindow>>,
    generation: Cell<u64>,
}

#[derive(Clone)]
pub struct SurfaceDismisser {
    state: Rc<DismissState>,
}

impl SurfaceDismisser {
    fn new() -> Self {
        Self {
            state: Rc::new(DismissState {
                close: RefCell::new(None),
                generation: Cell::new(0),
            }),
        }
    }

    pub fn dismiss(&self, cx: &mut App) {
        self.state
            .generation
            .set(self.state.generation.get().wrapping_add(1));
        if let Some(close) = self.state.close.borrow_mut().take() {
            cx.defer(move |cx| close(cx));
        }
    }
}

impl Surface {
    pub fn new(kind: SurfaceKind) -> Self {
        Self {
            kind,
            title: "qol-surface".into(),
            anchor: Anchor::CornerStack(Corner::BottomRight),
            timeout: None,
            size: size(px(320.0), px(72.0)),
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn size(mut self, size: Size<Pixels>) -> Self {
        self.size = size;
        self
    }

    pub fn show<V: Render + 'static>(
        self,
        tracker: &MonitorTracker,
        cx: &mut App,
        build: impl FnOnce(SurfaceDismisser, &mut Window, &mut Context<V>) -> V + 'static,
    ) -> Result<SurfaceDismisser> {
        self.open(tracker, cx, |dismisser, window, cx| {
            build(dismisser, window, cx)
        })
    }

    pub fn show_focused<V: Render + Focusable + 'static>(
        self,
        tracker: &MonitorTracker,
        cx: &mut App,
        build: impl FnOnce(SurfaceDismisser, &mut Window, &mut Context<V>) -> V + 'static,
    ) -> Result<SurfaceDismisser> {
        self.open(tracker, cx, |dismisser, window, cx| {
            let view = build(dismisser, window, cx);
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            view
        })
    }

    fn open<V: Render + 'static>(
        self,
        tracker: &MonitorTracker,
        cx: &mut App,
        build: impl FnOnce(SurfaceDismisser, &mut Window, &mut Context<V>) -> V + 'static,
    ) -> Result<SurfaceDismisser> {
        let monitor = match self.kind {
            SurfaceKind::Toast => tracker.snapshot_cursor().map(|(monitor, _)| monitor),
            SurfaceKind::Panel => tracker.snapshot_monitor(),
        }
        .ok_or_else(|| anyhow!("no monitor state available for surface placement"))?;
        let bounds = self.resolved_bounds(&monitor);
        let title = unique_surface_title(&self.title);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: crate::window::display_id_for_monitor(Some(&monitor), cx),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: self.window_kind(),
            focus: self.takes_focus(),
            is_movable: true,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some(title.clone()),
            ..Default::default()
        };
        let dismisser = SurfaceDismisser::new();
        let build_dismisser = dismisser.clone();
        let window_title = title.clone();
        let reveal_after_move = matches!(self.kind, SurfaceKind::Panel);
        let handle = cx.open_window(options, move |window, cx| {
            window.set_window_title(&window_title);
            let inner = cx.new(|cx| build(build_dismisser, window, cx));
            cx.new(|_| SurfaceRoot {
                inner,
                revealed: !reveal_after_move,
            })
        })?;
        dismisser
            .state
            .close
            .borrow_mut()
            .replace(Box::new(move |cx: &mut App| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }));
        if let Some(timeout) = self.timeout {
            schedule_dismiss(dismisser.clone(), timeout, cx);
        }
        if reveal_after_move {
            settle_then_reveal(handle, title, bounds.origin, cx);
        }
        Ok(dismisser)
    }

    fn resolved_bounds(&self, monitor: &crate::monitor::ActiveMonitor) -> Bounds<Pixels> {
        match self.anchor {
            Anchor::CornerStack(corner) => {
                corner_anchored_bounds(monitor.bounds(), corner, self.size, CORNER_MARGIN)
            }
            Anchor::MonitorCenter => {
                monitor.centered_bounds(clamped_to_monitor(self.size, monitor.bounds()))
            }
        }
    }

    fn window_kind(&self) -> WindowKind {
        match self.kind {
            SurfaceKind::Toast => WindowKind::PopUp,
            SurfaceKind::Panel => WindowKind::Normal,
        }
    }

    fn takes_focus(&self) -> bool {
        match self.kind {
            SurfaceKind::Toast => false,
            SurfaceKind::Panel => true,
        }
    }
}

struct SurfaceRoot<V> {
    inner: Entity<V>,
    revealed: bool,
}

impl<V: Render + 'static> Render for SurfaceRoot<V> {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div().size_full();
        if self.revealed {
            root = root.child(self.inner.clone());
        }
        root
    }
}

static SURFACE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static PANEL_FOCUS_GENERATION: AtomicU64 = AtomicU64::new(0);

fn unique_surface_title(base: &str) -> String {
    format!(
        "{base}-{}",
        SURFACE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn settle_then_reveal<V: Render + 'static>(
    handle: WindowHandle<SurfaceRoot<V>>,
    title: String,
    origin: Point<Pixels>,
    cx: &mut App,
) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        for _ in 0..40 {
            cx.background_executor()
                .timer(Duration::from_millis(15))
                .await;
            let moved = crate::popup_window::reposition_window_by_title(
                &title,
                origin.x.to_f64(),
                origin.y.to_f64(),
            );
            if moved {
                break;
            }
        }
        let _ = cx.update(|cx| {
            let _ = handle.update(cx, |root, _, cx| {
                root.revealed = true;
                cx.notify();
            });
        });
        let focus_commit = PANEL_FOCUS_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        crate::popup_window::reassert_focus_until_held(
            &title,
            &PANEL_FOCUS_GENERATION,
            focus_commit,
        );
        for _ in 0..3 {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            crate::popup_window::reposition_window_by_title(
                &title,
                origin.x.to_f64(),
                origin.y.to_f64(),
            );
        }
    })
    .detach();
}

fn schedule_dismiss(dismisser: SurfaceDismisser, timeout: Duration, cx: &mut App) {
    let scheduled = dismisser.state.generation.get();
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(timeout).await;
        if dismisser.state.generation.get() != scheduled {
            return;
        }
        let _ = cx.update(|cx| dismisser.dismiss(cx));
    })
    .detach();
}

fn clamped_to_monitor(win: Size<Pixels>, monitor: Bounds<Pixels>) -> Size<Pixels> {
    let margin = px(2.0 * CORNER_MARGIN);
    let max_width = monitor.size.width - margin;
    let max_height = monitor.size.height - margin;
    size(
        if win.width > max_width {
            max_width
        } else {
            win.width
        },
        if win.height > max_height {
            max_height
        } else {
            win.height
        },
    )
}

fn corner_anchored_bounds(
    monitor: Bounds<Pixels>,
    corner: Corner,
    win: Size<Pixels>,
    margin: f32,
) -> Bounds<Pixels> {
    let min_x = monitor.origin.x.to_f64() as f32 + margin;
    let max_x =
        ((monitor.origin.x + monitor.size.width - win.width).to_f64() as f32 - margin).max(min_x);
    let min_y = monitor.origin.y.to_f64() as f32 + margin;
    let max_y =
        ((monitor.origin.y + monitor.size.height - win.height).to_f64() as f32 - margin).max(min_y);
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => min_x,
        Corner::TopRight | Corner::BottomRight => max_x,
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => min_y,
        Corner::BottomLeft | Corner::BottomRight => max_y,
    };
    Bounds::new(point(px(x), px(y)), win)
}

#[cfg(test)]
mod tests {
    use super::{corner_anchored_bounds, Corner};
    use gpui::{point, px, size, Bounds};

    #[test]
    fn corner_anchored_bounds_places_each_corner_inside_margins() {
        let monitor = Bounds::new(point(px(1920.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let win = size(px(340.0), px(76.0));
        let cases = [
            (Corner::TopLeft, (1944.0, 24.0)),
            (Corner::TopRight, (4116.0, 24.0)),
            (Corner::BottomLeft, (1944.0, 1340.0)),
            (Corner::BottomRight, (4116.0, 1340.0)),
        ];

        for (corner, expected) in cases {
            let bounds = corner_anchored_bounds(monitor, corner, win, 24.0);
            assert_eq!(
                (
                    bounds.origin.x.to_f64() as f32,
                    bounds.origin.y.to_f64() as f32
                ),
                expected,
                "corner: {corner:?}"
            );
        }
    }

    #[test]
    fn corner_anchored_bounds_supports_negative_origins_and_tiny_monitors() {
        let win = size(px(340.0), px(76.0));

        let negative = corner_anchored_bounds(
            Bounds::new(point(px(-1920.0), px(-200.0)), size(px(1920.0), px(1080.0))),
            Corner::BottomRight,
            win,
            24.0,
        );
        assert_eq!(negative.origin.x.to_f64(), -364.0);
        assert_eq!(negative.origin.y.to_f64(), 780.0);

        let tiny = corner_anchored_bounds(
            Bounds::new(point(px(0.0), px(0.0)), size(px(200.0), px(50.0))),
            Corner::BottomRight,
            win,
            24.0,
        );
        assert_eq!(tiny.origin.x.to_f64(), 24.0);
        assert_eq!(tiny.origin.y.to_f64(), 24.0);
    }
}
