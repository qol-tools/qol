use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, Result};
use gpui::*;

use crate::monitor::MonitorTracker;

pub const CORNER_MARGIN: f32 = 24.0;
const REUSED_REVEAL_SAMPLE_INTERVAL: Duration = Duration::from_millis(5);
const REUSED_REVEAL_MAX_ATTEMPTS: usize = 100;

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
    retain_on_dismiss: bool,
}

pub(crate) struct OpenedSurface<V> {
    pub(crate) handle: WindowHandle<SurfaceRoot<V>>,
    pub(crate) dismisser: SurfaceDismisser,
    title: String,
    anchor: Anchor,
    size: Size<Pixels>,
    visible: Rc<Cell<bool>>,
    reveal_pending: Rc<Cell<bool>>,
}

type CloseWindow = Box<dyn Fn(&mut App)>;

struct DismissState {
    close: RefCell<Option<CloseWindow>>,
    generation: Cell<u64>,
    reusable: bool,
}

#[derive(Clone)]
pub struct SurfaceDismisser {
    state: Rc<DismissState>,
}

impl SurfaceDismisser {
    fn new(reusable: bool) -> Self {
        Self {
            state: Rc::new(DismissState {
                close: RefCell::new(None),
                generation: Cell::new(0),
                reusable,
            }),
        }
    }

    pub fn dismiss(&self, cx: &mut App) {
        self.state
            .generation
            .set(self.state.generation.get().wrapping_add(1));
        if self.state.reusable {
            let state = self.state.clone();
            cx.defer(move |cx| {
                if let Some(close) = state.close.borrow().as_ref() {
                    close(cx);
                }
            });
            return;
        }
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
            retain_on_dismiss: false,
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

    pub(crate) fn retain_on_dismiss(mut self) -> Self {
        self.retain_on_dismiss = true;
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
        .map(|opened| opened.dismisser)
    }

    pub fn show_focused<V: Render + Focusable + 'static>(
        self,
        tracker: &MonitorTracker,
        cx: &mut App,
        build: impl FnOnce(SurfaceDismisser, &mut Window, &mut Context<V>) -> V + 'static,
    ) -> Result<SurfaceDismisser> {
        self.show_focused_tracked(tracker, cx, build)
            .map(|opened| opened.dismisser)
    }

    pub(crate) fn show_focused_tracked<V: Render + Focusable + 'static>(
        self,
        tracker: &MonitorTracker,
        cx: &mut App,
        build: impl FnOnce(SurfaceDismisser, &mut Window, &mut Context<V>) -> V + 'static,
    ) -> Result<OpenedSurface<V>> {
        self.open(tracker, cx, |dismisser, window, cx| {
            let view = build(dismisser, window, cx);
            window.focus(&view.focus_handle(cx));
            view
        })
    }

    fn open<V: Render + 'static>(
        self,
        tracker: &MonitorTracker,
        cx: &mut App,
        build: impl FnOnce(SurfaceDismisser, &mut Window, &mut Context<V>) -> V + 'static,
    ) -> Result<OpenedSurface<V>> {
        let monitor = match self.kind {
            SurfaceKind::Toast => tracker.snapshot_cursor().map(|(monitor, _)| monitor),
            SurfaceKind::Panel => tracker.snapshot_monitor(),
        }
        .ok_or_else(|| anyhow!("no monitor state available for surface placement"))?;
        let bounds = self.resolved_bounds(&monitor);
        let title = unique_surface_title(&self.title);
        let reveal_after_move = matches!(self.kind, SurfaceKind::Panel);
        let native_reveal_gate = reveal_after_move && supports_native_reveal_gate();
        let retain_on_dismiss = self.retain_on_dismiss && native_reveal_gate;
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: crate::window::display_id_for_monitor(Some(&monitor), cx),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: self.window_kind(),
            focus: self.takes_focus(),
            show: !native_reveal_gate,
            is_movable: true,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some(title.clone()),
            ..Default::default()
        };
        let dismisser = SurfaceDismisser::new(retain_on_dismiss);
        let build_dismisser = dismisser.clone();
        let window_title = title.clone();
        let visible = Rc::new(Cell::new(!native_reveal_gate));
        let reveal_pending = Rc::new(Cell::new(native_reveal_gate));
        let handle = cx.open_window(options, move |window, cx| {
            window.set_window_title(&window_title);
            let inner = cx.new(|cx| build(build_dismisser, window, cx));
            cx.new(|_| SurfaceRoot {
                inner,
                render_epoch: Rc::new(Cell::new(0)),
            })
        })?;
        let dismiss_title = title.clone();
        let dismiss_visible = visible.clone();
        let dismiss_reveal_pending = reveal_pending.clone();
        dismisser
            .state
            .close
            .borrow_mut()
            .replace(Box::new(move |cx: &mut App| {
                dismiss_visible.set(false);
                dismiss_reveal_pending.set(false);
                if retain_on_dismiss {
                    let _reason = crate::popup_window::reason_scope("surface-dismiss");
                    if crate::popup_window::park_window_by_title(&dismiss_title) {
                        return;
                    }
                }
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }));
        if let Some(timeout) = self.timeout {
            schedule_dismiss(dismisser.clone(), timeout, cx);
        }
        if native_reveal_gate {
            let _reason = crate::popup_window::reason_scope("surface-open");
            let hidden = crate::popup_window::prepare_window_reveal_by_title(&title);
            let fresh_frame = hidden.then(|| schedule_fresh_frame(handle, cx)).flatten();
            let frame_scheduled = fresh_frame.is_some();
            qol_runtime::probe!(
                "SURFACE_REVEAL",
                "title={title} phase=opened hidden={hidden} frame_scheduled={frame_scheduled} x={} y={}",
                bounds.origin.x.to_f64(),
                bounds.origin.y.to_f64()
            );
            if !frame_scheduled {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
                return Err(anyhow!("surface could not prepare a fresh frame"));
            }
            settle_then_reveal(
                PendingReveal {
                    handle,
                    title: title.clone(),
                    origin: bounds.origin,
                    visible: visible.clone(),
                    reveal_pending: reveal_pending.clone(),
                    fresh_frame: fresh_frame.expect("fresh frame was scheduled"),
                    dismiss_generation: dismisser.state.generation.get(),
                    dismiss_state: dismisser.state.clone(),
                },
                cx,
            );
        }
        Ok(OpenedSurface {
            handle,
            dismisser,
            title,
            anchor: self.anchor,
            size: self.size,
            visible,
            reveal_pending,
        })
    }

    fn resolved_bounds(&self, monitor: &crate::monitor::ActiveMonitor) -> Bounds<Pixels> {
        resolved_bounds(self.anchor, self.size, monitor)
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

pub(crate) struct SurfaceRoot<V> {
    pub(crate) inner: Entity<V>,
    render_epoch: Rc<Cell<u64>>,
}

impl<V: Render + 'static> Render for SurfaceRoot<V> {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.render_epoch
            .set(self.render_epoch.get().wrapping_add(1));
        div().size_full().child(self.inner.clone())
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

const fn supports_native_reveal_gate() -> bool {
    cfg!(any(target_os = "linux", target_os = "macos"))
}

struct PendingReveal<V: Render + 'static> {
    handle: WindowHandle<SurfaceRoot<V>>,
    title: String,
    origin: Point<Pixels>,
    visible: Rc<Cell<bool>>,
    reveal_pending: Rc<Cell<bool>>,
    fresh_frame: FreshFrame,
    dismiss_generation: u64,
    dismiss_state: Rc<DismissState>,
}

fn settle_then_reveal<V: Render + 'static>(pending: PendingReveal<V>, cx: &mut App) {
    let PendingReveal {
        handle,
        title,
        origin,
        visible,
        reveal_pending,
        fresh_frame,
        dismiss_generation,
        dismiss_state,
    } = pending;
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (readiness, attempts) =
            await_reveal_readiness(cx, &title, origin, &fresh_frame).await;
        let window_exists = cx.update(|cx| handle.update(cx, |_, _, _| ()).is_ok());
        if !matches!(window_exists, Ok(true)) {
            reveal_pending.set(false);
            return;
        }
        if dismiss_state.generation.get() != dismiss_generation {
            reveal_pending.set(false);
            return;
        }
        qol_runtime::probe!(
            "SURFACE_REVEAL",
            "title={title} phase=frame-ready moved={} fresh_frame={} content_rendered={} attempts={attempts}",
            readiness.moved,
            readiness.fresh_frame,
            readiness.content_rendered
        );
        if !readiness.ready() {
            reveal_pending.set(false);
            let _ = cx.update(|cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
            qol_runtime::probe!(
                "SURFACE_REVEAL",
                "title={title} phase=revealed moved={} fresh_frame={} content_rendered={} attempts={attempts} shown=false reason=frame-not-ready",
                readiness.moved,
                readiness.fresh_frame,
                readiness.content_rendered
            );
            return;
        }
        let shown = {
            let _reason = crate::popup_window::reason_scope("surface-reveal");
            crate::popup_window::show_window_by_title(&title)
        };
        let repaint_requested = shown
            && cx
                .update(|cx| request_surface_repaint(handle, cx))
                .unwrap_or(false);
        visible.set(shown);
        reveal_pending.set(false);
        qol_runtime::probe!(
            "SURFACE_REVEAL",
            "title={title} phase=revealed moved={} fresh_frame={} content_rendered={} attempts={attempts} shown={shown} repaint_requested={repaint_requested}",
            readiness.moved,
            readiness.fresh_frame,
            readiness.content_rendered
        );
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RevealReadiness {
    moved: bool,
    fresh_frame: bool,
    content_rendered: bool,
}

impl RevealReadiness {
    fn ready(self) -> bool {
        self.moved && self.fresh_frame && self.content_rendered
    }
}

#[derive(Clone)]
struct FreshFrame {
    presented: Rc<Cell<bool>>,
    render_epoch: Rc<Cell<u64>>,
    required_render_epoch: u64,
}

impl FreshFrame {
    fn presented(&self) -> bool {
        self.presented.get()
    }

    fn content_rendered(&self) -> bool {
        self.render_epoch.get() >= self.required_render_epoch
    }
}

fn schedule_fresh_frame<V: Render + 'static>(
    handle: WindowHandle<SurfaceRoot<V>>,
    cx: &mut App,
) -> Option<FreshFrame> {
    let presented = Rc::new(Cell::new(false));
    let presented_after_frame = presented.clone();
    let mut request = None;
    handle
        .update(cx, |root, window, cx| {
            request = Some(FreshFrame {
                presented,
                render_epoch: root.render_epoch.clone(),
                required_render_epoch: root.render_epoch.get().wrapping_add(1),
            });
            window.on_next_frame(move |window, _| {
                window.on_next_frame(move |_, _| presented_after_frame.set(true));
                window.refresh();
            });
            root.inner.update(cx, |_, cx| cx.notify());
            cx.notify();
            window.refresh();
        })
        .ok()?;
    request
}

fn request_surface_repaint<V: Render + 'static>(
    handle: WindowHandle<SurfaceRoot<V>>,
    cx: &mut App,
) -> bool {
    handle
        .update(cx, |root, window, cx| {
            root.inner.update(cx, |_, cx| cx.notify());
            cx.notify();
            window.refresh();
        })
        .is_ok()
}

async fn await_reveal_readiness(
    cx: &mut AsyncApp,
    title: &str,
    origin: Point<Pixels>,
    fresh_frame: &FreshFrame,
) -> (RevealReadiness, usize) {
    let mut readiness = RevealReadiness::default();
    for attempt in 1..=40 {
        cx.background_executor()
            .timer(Duration::from_millis(15))
            .await;
        readiness.moved |= crate::popup_window::reposition_window_by_title(
            title,
            origin.x.to_f64(),
            origin.y.to_f64(),
        );
        readiness.fresh_frame = fresh_frame.presented();
        readiness.content_rendered = fresh_frame.content_rendered();
        if readiness.ready() {
            return (readiness, attempt);
        }
    }
    (readiness, 40)
}

impl<V: Render + Focusable + 'static> OpenedSurface<V> {
    pub(crate) fn is_visible(&self) -> bool {
        self.visible.get()
    }

    pub(crate) fn present(&mut self, tracker: &MonitorTracker, cx: &mut App) -> bool {
        if self.handle.update(cx, |_, _, _| ()).is_err() {
            return false;
        }
        if !self.visible.get() {
            if self.reveal_pending.get() {
                return true;
            }
            let Some(monitor) = tracker.snapshot_monitor() else {
                return false;
            };
            let bounds = resolved_bounds(self.anchor, self.size, &monitor);
            let prepared = {
                let _reason = crate::popup_window::reason_scope("surface-reuse");
                crate::popup_window::prepare_window_reveal_by_title(&self.title)
            };
            if !prepared {
                return false;
            }
            let Some(fresh_frame) = schedule_fresh_frame(self.handle, cx) else {
                let _ = crate::popup_window::park_window_by_title(&self.title);
                return false;
            };
            self.reveal_pending.set(true);
            qol_runtime::probe!(
                "SURFACE_REVEAL",
                "title={} phase=opened hidden=true frame_scheduled=true reused=true x={} y={}",
                self.title,
                bounds.origin.x.to_f64(),
                bounds.origin.y.to_f64()
            );
            settle_then_reveal_reused(
                PendingReveal {
                    handle: self.handle,
                    title: self.title.clone(),
                    origin: bounds.origin,
                    visible: self.visible.clone(),
                    reveal_pending: self.reveal_pending.clone(),
                    fresh_frame,
                    dismiss_generation: self.dismisser.state.generation.get(),
                    dismiss_state: self.dismisser.state.clone(),
                },
                cx,
            );
            return true;
        }
        self.handle
            .update(cx, |root, window, cx| {
                let focus = root.inner.read(cx).focus_handle(cx);
                window.activate_window();
                window.focus(&focus);
            })
            .is_ok()
    }

    pub(crate) fn resize(&mut self, size: Size<Pixels>) {
        self.size = size;
    }
}

fn settle_then_reveal_reused<V: Render + Focusable + 'static>(
    pending: PendingReveal<V>,
    cx: &mut App,
) {
    let PendingReveal {
        handle,
        title,
        origin,
        visible,
        reveal_pending,
        fresh_frame,
        dismiss_generation,
        dismiss_state,
    } = pending;
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (readiness, attempts) =
            await_reveal_readiness(cx, &title, origin, &fresh_frame).await;
        let window_exists = cx.update(|cx| handle.update(cx, |_, _, _| ()).is_ok());
        if !matches!(window_exists, Ok(true)) {
            reveal_pending.set(false);
            return;
        }
        if dismiss_state.generation.get() != dismiss_generation {
            reveal_pending.set(false);
            return;
        }
        qol_runtime::probe!(
            "SURFACE_REVEAL",
            "title={title} phase=frame-ready moved={} fresh_frame={} content_rendered={} attempts={attempts} reused=true",
            readiness.moved,
            readiness.fresh_frame,
            readiness.content_rendered
        );
        if !readiness.ready() {
            reveal_pending.set(false);
            let _reason = crate::popup_window::reason_scope("surface-reuse-timeout");
            let _ = crate::popup_window::park_window_by_title(&title);
            qol_runtime::probe!(
                "SURFACE_REVEAL",
                "title={title} phase=revealed moved={} fresh_frame={} content_rendered={} attempts={attempts} shown=false reused=true reason=frame-not-ready",
                readiness.moved,
                readiness.fresh_frame,
                readiness.content_rendered
            );
            return;
        }
        let shown = {
            let _reason = crate::popup_window::reason_scope("surface-reuse-reveal");
            crate::popup_window::show_window_by_title(&title)
        };
        visible.set(shown);
        reveal_pending.set(false);
        if !shown {
            return;
        }
        let repaint_requested = cx
            .update(|cx| {
                handle
                    .update(cx, |root, window, cx| {
                        let focus = root.inner.read(cx).focus_handle(cx);
                        window.activate_window();
                        window.focus(&focus);
                        root.inner.update(cx, |_, cx| cx.notify());
                        cx.notify();
                        window.refresh();
                    })
                    .is_ok()
            })
            .unwrap_or(false);
        qol_runtime::probe!(
            "SURFACE_REVEAL",
            "title={title} phase=revealed moved={} fresh_frame={} content_rendered={} attempts={attempts} shown=true reused=true repaint_requested={repaint_requested}",
            readiness.moved,
            readiness.fresh_frame,
            readiness.content_rendered
        );
        let focus_commit = PANEL_FOCUS_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        crate::popup_window::reassert_focus_until_held(
            &title,
            &PANEL_FOCUS_GENERATION,
            focus_commit,
        );
        let _ = cx.update(|cx| trace_reused_ready(title, cx));
    })
    .detach();
}

fn trace_reused_ready(title: String, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        for attempt in 0..=REUSED_REVEAL_MAX_ATTEMPTS {
            let visible = cx
                .update(|_| crate::popup_window::visible_windows_by_title_prefix(&title))
                .unwrap_or_default()
                > 0;
            let focused = visible
                && cx
                    .update(|_| crate::popup_window::window_holds_input_focus(&title))
                    .ok()
                    .flatten()
                    .unwrap_or(false);
            if focused {
                qol_runtime::probe!(
                    "SURFACE_REVEAL",
                    "title={title} phase=ready focus=true attempts={attempt} reused=true"
                );
                return;
            }
            cx.background_executor()
                .timer(REUSED_REVEAL_SAMPLE_INTERVAL)
                .await;
        }
        qol_runtime::probe!(
            "SURFACE_REVEAL",
            "title={title} phase=ready focus=false attempts={REUSED_REVEAL_MAX_ATTEMPTS} reused=true"
        );
    })
    .detach();
}

fn resolved_bounds(
    anchor: Anchor,
    size: Size<Pixels>,
    monitor: &crate::monitor::ActiveMonitor,
) -> Bounds<Pixels> {
    match anchor {
        Anchor::CornerStack(corner) => {
            corner_anchored_bounds(monitor.bounds(), corner, size, CORNER_MARGIN)
        }
        Anchor::MonitorCenter => {
            monitor.centered_bounds(clamped_to_monitor(size, monitor.bounds()))
        }
    }
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
    use super::{corner_anchored_bounds, Corner, RevealReadiness};
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

    #[test]
    fn reveal_requires_placement_rendered_content_and_a_presented_frame() {
        let cases = [
            (false, false, false, false),
            (true, false, true, false),
            (true, true, false, false),
            (false, true, true, false),
            (true, true, true, true),
        ];
        for (moved, fresh_frame, content_rendered, expected) in cases {
            assert_eq!(
                RevealReadiness {
                    moved,
                    fresh_frame,
                    content_rendered,
                }
                .ready(),
                expected,
                "moved={moved} fresh_frame={fresh_frame} content_rendered={content_rendered}"
            );
        }
    }
}
