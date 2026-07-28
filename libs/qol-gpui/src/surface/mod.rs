use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, Result};
use gpui::*;

use crate::monitor::MonitorTracker;
use crate::placement::{Corner, MonitorPlacement, CORNER_MARGIN};

mod platform;

use self::platform::{Platform, SurfacePlatform};

const REUSED_REVEAL_SAMPLE_INTERVAL: Duration = Duration::from_millis(5);
const REUSED_REVEAL_MAX_ATTEMPTS: usize = 100;
const VIEWPORT_TOLERANCE: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    Toast,
    Panel,
}

/// Marks non-interactive panel chrome as a native window drag area.
///
/// The surface root cannot safely own this listener because mouse events from
/// interactive children bubble through it. Views should apply this only to
/// their semantic top chrome.
pub trait PanelDragArea: InteractiveElement + Sized {
    fn panel_drag_area(self) -> Self {
        self.on_mouse_down(MouseButton::Left, |_, window, _| {
            window.start_window_move();
        })
    }
}

impl<T: InteractiveElement> PanelDragArea for T {}

pub struct Surface {
    kind: SurfaceKind,
    title: String,
    placement: MonitorPlacement,
    timeout: Option<Duration>,
    size: Size<Pixels>,
    retain_on_dismiss: bool,
}

pub struct OpenedSurface<V> {
    pub(crate) handle: WindowHandle<SurfaceRoot<V>>,
    pub(crate) dismisser: SurfaceDismisser,
    placement: MonitorPlacement,
    size: Size<Pixels>,
    constrains_size: bool,
    visible: Rc<Cell<bool>>,
    reveal_pending: Rc<Cell<bool>>,
}

type CloseWindow = Box<dyn Fn(&mut App)>;

struct DismissState {
    close: RefCell<Option<CloseWindow>>,
    generation: Cell<u64>,
    reusable: bool,
    title: RefCell<String>,
}

#[derive(Clone)]
pub struct SurfaceDismisser {
    state: Rc<DismissState>,
}

impl SurfaceDismisser {
    fn new(reusable: bool, title: String) -> Self {
        Self {
            state: Rc::new(DismissState {
                close: RefCell::new(None),
                generation: Cell::new(0),
                reusable,
                title: RefCell::new(title),
            }),
        }
    }

    pub(crate) fn current_title(&self) -> String {
        self.state.title.borrow().clone()
    }

    pub(crate) fn retitle(&self, window: &mut Window, title: String) {
        window.set_window_title(&title);
        *self.state.title.borrow_mut() = title;
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
        let placement = match kind {
            SurfaceKind::Toast => MonitorPlacement::corner(Corner::BottomRight, CORNER_MARGIN),
            SurfaceKind::Panel => MonitorPlacement::center(),
        };
        Self {
            kind,
            title: "qol-surface".into(),
            placement,
            timeout: None,
            size: size(px(320.0), px(72.0)),
            retain_on_dismiss: false,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn placement(mut self, placement: MonitorPlacement) -> Self {
        self.placement = placement;
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
    ) -> Result<OpenedSurface<V>> {
        self.open(tracker, cx, |dismisser, window, cx| {
            let view = build(dismisser, window, cx);
            window.focus(&view.focus_handle(cx));
            view
        })
    }

    pub(crate) fn open<V: Render + 'static>(
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
        let constrains_size = self.constrains_size();
        let reveal_after_move = matches!(self.kind, SurfaceKind::Panel);
        let native_reveal_gate = reveal_after_move && supports_native_reveal_gate();
        let passive_reveal_gate =
            matches!(self.kind, SurfaceKind::Toast) && supports_native_reveal_gate();
        let retain_on_dismiss = self.retain_on_dismiss && native_reveal_gate;
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: crate::window::display_id_for_monitor(Some(&monitor), cx),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: self.window_kind(),
            focus: self.takes_focus(),
            show: !native_reveal_gate && !passive_reveal_gate,
            is_movable: true,
            is_resizable: !constrains_size,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some(title.clone()),
            ..Default::default()
        };
        let dismisser = SurfaceDismisser::new(retain_on_dismiss, title.clone());
        let build_dismisser = dismisser.clone();
        let window_title = title.clone();
        let visible = Rc::new(Cell::new(!native_reveal_gate && !passive_reveal_gate));
        let reveal_pending = Rc::new(Cell::new(native_reveal_gate));
        if self.takes_focus() {
            crate::popup_window::capture_focus_return();
        }
        let handle = cx.open_window(options, move |window, cx| {
            window.set_window_title(&window_title);
            let inner = cx.new(|cx| build(build_dismisser, window, cx));
            cx.new(|cx| {
                let bounds_subscription =
                    cx.observe_window_bounds(window, |root: &mut SurfaceRoot<V>, window, cx| {
                        root.layout_epoch
                            .set(root.layout_epoch.get().wrapping_add(1));
                        root.observed_viewport.set(window.viewport_size());
                        cx.notify();
                    });
                SurfaceRoot {
                    inner,
                    render_epoch: Rc::new(Cell::new(0)),
                    layout_epoch: Rc::new(Cell::new(0)),
                    rendered_layout_epoch: Rc::new(Cell::new(0)),
                    observed_viewport: Rc::new(Cell::new(size(px(0.0), px(0.0)))),
                    rendered_viewport: Rc::new(Cell::new(size(px(0.0), px(0.0)))),
                    _bounds_subscription: bounds_subscription,
                }
            })
        })?;
        let dismiss_state = dismisser.state.clone();
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
                    let current_title = dismiss_state.title.borrow().clone();
                    let _reason = crate::popup_window::reason_scope("surface-dismiss");
                    crate::popup_window::set_window_type_dock_by_title(&current_title);
                    if crate::popup_window::hide_invisible(&current_title) {
                        return;
                    }
                }
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }));
        if let Some(timeout) = self.timeout {
            schedule_dismiss(dismisser.clone(), timeout, cx);
        }
        if passive_reveal_gate {
            let _reason = crate::popup_window::reason_scope("surface-toast");
            let configured = crate::popup_window::configure_popup_window(&title);
            if configured {
                crate::popup_window::present_topmost(&title);
            }
            let shown = configured && crate::popup_window::show_window_passive_by_title(&title);
            visible.set(shown);
            qol_runtime::probe!(
                "SURFACE_REVEAL",
                "title={title} phase=toast-ready configured={configured} shown={shown}"
            );
            if !shown {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
                return Err(anyhow!("surface could not present passive toast"));
            }
        }
        if native_reveal_gate {
            let _reason = crate::popup_window::reason_scope("surface-open");
            let hidden = crate::popup_window::prepare_window_reveal_by_title(&title);
            let size_constrained = !constrains_size || constrain_native_size(&title, bounds.size);
            let fresh_frame = (hidden && size_constrained)
                .then(|| schedule_fresh_frame(handle, bounds.size, cx))
                .flatten();
            let frame_scheduled = fresh_frame.is_some();
            qol_runtime::probe!(
                "SURFACE_REVEAL",
                "title={title} phase=opened hidden={hidden} fixed_size={} size_constrained={size_constrained} frame_scheduled={frame_scheduled} x={} y={}",
                constrains_size,
                bounds.origin.x.to_f64(),
                bounds.origin.y.to_f64()
            );
            if !frame_scheduled {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
                if !size_constrained {
                    return Err(anyhow!(
                        "surface could not constrain its native window size"
                    ));
                }
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
            placement: self.placement,
            size: self.size,
            constrains_size,
            visible,
            reveal_pending,
        })
    }

    fn resolved_bounds(&self, monitor: &crate::monitor::ActiveMonitor) -> Bounds<Pixels> {
        self.placement.bounds(monitor.bounds(), self.size)
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

    fn constrains_size(&self) -> bool {
        matches!(self.kind, SurfaceKind::Panel)
    }
}

pub(crate) struct SurfaceRoot<V> {
    pub(crate) inner: Entity<V>,
    render_epoch: Rc<Cell<u64>>,
    layout_epoch: Rc<Cell<u64>>,
    rendered_layout_epoch: Rc<Cell<u64>>,
    observed_viewport: Rc<Cell<Size<Pixels>>>,
    rendered_viewport: Rc<Cell<Size<Pixels>>>,
    _bounds_subscription: Subscription,
}

impl<V: Render + 'static> Render for SurfaceRoot<V> {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.render_epoch
            .set(self.render_epoch.get().wrapping_add(1));
        self.rendered_layout_epoch.set(self.layout_epoch.get());
        self.rendered_viewport.set(window.viewport_size());
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

fn supports_native_reveal_gate() -> bool {
    Platform::supports_native_reveal_gate()
}

fn constrain_native_size(title: &str, size: Size<Pixels>) -> bool {
    !supports_native_reveal_gate()
        || crate::popup_window::set_window_fixed_size_by_title(title, size)
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
            await_reveal_readiness(cx, handle, &title, origin, &fresh_frame).await;
        #[cfg(not(debug_assertions))]
        let _ = &attempts;
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
            "title={title} phase=frame-ready moved={} layout_confirmed={} viewport_ready={} fresh_frame={} content_rendered={} attempts={attempts} layout_epoch={}/{} render_epoch={}/{} gpu_completed_epoch={} expected={}x{} observed={}x{} rendered={}x{}",
            readiness.moved,
            readiness.layout_confirmed,
            readiness.viewport_ready,
            readiness.fresh_frame,
            readiness.content_rendered,
            fresh_frame.layout_epoch.get(),
            fresh_frame.required_layout_epoch,
            fresh_frame.render_epoch.get(),
            fresh_frame.required_render_epoch,
            fresh_frame.presented_render_epoch.get(),
            fresh_frame.expected_viewport.width.to_f64(),
            fresh_frame.expected_viewport.height.to_f64(),
            fresh_frame.observed_viewport.get().width.to_f64(),
            fresh_frame.observed_viewport.get().height.to_f64(),
            fresh_frame.rendered_viewport.get().width.to_f64(),
            fresh_frame.rendered_viewport.get().height.to_f64()
        );
        if !readiness.ready() {
            reveal_pending.set(false);
            let _ = cx.update(|cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
            qol_runtime::probe!(
                "SURFACE_REVEAL",
                "title={title} phase=revealed moved={} layout_confirmed={} viewport_ready={} fresh_frame={} content_rendered={} attempts={attempts} shown=false reason=frame-not-ready",
                readiness.moved,
                readiness.layout_confirmed,
                readiness.viewport_ready,
                readiness.fresh_frame,
                readiness.content_rendered
            );
            return;
        }
        let shown = {
            let _reason = crate::popup_window::reason_scope("surface-reveal");
            crate::popup_window::show_normal_window_by_title(&title)
        };
        let repaint_requested = shown
            && cx
                .update(|cx| request_surface_repaint(handle, cx))
                .unwrap_or(false);
        #[cfg(not(debug_assertions))]
        let _ = &repaint_requested;
        visible.set(shown);
        reveal_pending.set(false);
        qol_runtime::probe!(
            "SURFACE_REVEAL",
            "title={title} phase=revealed moved={} layout_confirmed={} viewport_ready={} fresh_frame={} content_rendered={} attempts={attempts} shown={shown} repaint_requested={repaint_requested}",
            readiness.moved,
            readiness.layout_confirmed,
            readiness.viewport_ready,
            readiness.fresh_frame,
            readiness.content_rendered
        );
        let focus_commit = PANEL_FOCUS_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        crate::popup_window::reassert_normal_focus_until_held(
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
    layout_confirmed: bool,
    viewport_ready: bool,
    fresh_frame: bool,
    content_rendered: bool,
}

impl RevealReadiness {
    fn ready(self) -> bool {
        self.moved
            && self.layout_confirmed
            && self.viewport_ready
            && self.fresh_frame
            && self.content_rendered
    }
}

#[derive(Clone)]
struct FreshFrame {
    render_epoch: Rc<Cell<u64>>,
    required_render_epoch: u64,
    layout_epoch: Rc<Cell<u64>>,
    required_layout_epoch: u64,
    rendered_layout_epoch: Rc<Cell<u64>>,
    observed_viewport: Rc<Cell<Size<Pixels>>>,
    rendered_viewport: Rc<Cell<Size<Pixels>>>,
    expected_viewport: Size<Pixels>,
    presented_render_epoch: Rc<Cell<u64>>,
    presented_layout_epoch: Rc<Cell<u64>>,
}

impl FreshFrame {
    fn layout_confirmed(&self) -> bool {
        Platform::layout_confirmed(
            self.layout_epoch.get(),
            self.required_layout_epoch,
            self.observed_viewport.get(),
            self.expected_viewport,
            VIEWPORT_TOLERANCE,
        )
    }

    fn viewport_ready(&self) -> bool {
        viewport_matches(self.rendered_viewport.get(), self.expected_viewport)
    }

    fn presented(&self) -> bool {
        self.presented_render_epoch.get() >= self.required_render_epoch
            && self.presented_layout_epoch.get() >= self.required_layout_epoch
    }

    fn content_rendered(&self) -> bool {
        self.render_epoch.get() >= self.required_render_epoch
            && self.rendered_layout_epoch.get() >= self.required_layout_epoch
            && self.viewport_ready()
    }

    fn request<V: Render + 'static>(
        &self,
        root: &mut SurfaceRoot<V>,
        window: &mut Window,
        cx: &mut Context<SurfaceRoot<V>>,
    ) {
        let render_epoch = self.render_epoch.clone();
        let rendered_layout_epoch = self.rendered_layout_epoch.clone();
        let presented_render_epoch = self.presented_render_epoch.clone();
        let presented_layout_epoch = self.presented_layout_epoch.clone();
        window.on_next_frame(move |_, _| {
            presented_render_epoch.set(render_epoch.get());
            presented_layout_epoch.set(rendered_layout_epoch.get());
        });
        root.inner.update(cx, |_, cx| cx.notify());
        cx.notify();
        window.refresh();
    }
}

fn schedule_fresh_frame<V: Render + 'static>(
    handle: WindowHandle<SurfaceRoot<V>>,
    expected_viewport: Size<Pixels>,
    cx: &mut App,
) -> Option<FreshFrame> {
    let mut request = None;
    handle
        .update(cx, |root, window, cx| {
            let fresh_frame = FreshFrame {
                render_epoch: root.render_epoch.clone(),
                required_render_epoch: root.render_epoch.get().wrapping_add(1),
                layout_epoch: root.layout_epoch.clone(),
                required_layout_epoch: required_layout_epoch(root.layout_epoch.get()),
                rendered_layout_epoch: root.rendered_layout_epoch.clone(),
                observed_viewport: root.observed_viewport.clone(),
                rendered_viewport: root.rendered_viewport.clone(),
                expected_viewport,
                presented_render_epoch: Rc::new(Cell::new(0)),
                presented_layout_epoch: Rc::new(Cell::new(0)),
            };
            fresh_frame.request(root, window, cx);
            request = Some(fresh_frame);
        })
        .ok()?;
    request
}

fn required_layout_epoch(current: u64) -> u64 {
    Platform::required_layout_epoch(current)
}

fn viewport_matches(actual: Size<Pixels>, expected: Size<Pixels>) -> bool {
    (actual.width.to_f64() - expected.width.to_f64()).abs() <= VIEWPORT_TOLERANCE
        && (actual.height.to_f64() - expected.height.to_f64()).abs() <= VIEWPORT_TOLERANCE
}

fn request_pending_frame<V: Render + 'static>(
    handle: WindowHandle<SurfaceRoot<V>>,
    fresh_frame: &FreshFrame,
    cx: &mut App,
) -> bool {
    handle
        .update(cx, |root, window, cx| {
            fresh_frame.request(root, window, cx);
        })
        .is_ok()
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

async fn await_reveal_readiness<V: Render + 'static>(
    cx: &mut AsyncApp,
    handle: WindowHandle<SurfaceRoot<V>>,
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
        readiness.layout_confirmed = fresh_frame.layout_confirmed();
        readiness.viewport_ready = fresh_frame.viewport_ready();
        readiness.fresh_frame = fresh_frame.presented();
        readiness.content_rendered = fresh_frame.content_rendered();
        if readiness.ready() {
            return (readiness, attempt);
        }
        let frame_requested = cx
            .update(|cx| request_pending_frame(handle, fresh_frame, cx))
            .unwrap_or(false);
        if !frame_requested {
            return (readiness, attempt);
        }
    }
    (readiness, 40)
}

impl<V: Render + Focusable + 'static> OpenedSurface<V> {
    pub(crate) fn is_visible(&self) -> bool {
        self.visible.get()
    }

    pub fn present(&mut self, tracker: &MonitorTracker, cx: &mut App) -> bool {
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
            let bounds = self.placement.bounds(monitor.bounds(), self.size);
            crate::popup_window::capture_focus_return();
            let title = self.dismisser.current_title();
            if self.constrains_size && !constrain_native_size(&title, bounds.size) {
                return false;
            }
            let resized = self
                .handle
                .update(cx, |_, window, _| window.resize(self.size))
                .is_ok();
            if !resized {
                return false;
            }
            let prepared = {
                let _reason = crate::popup_window::reason_scope("surface-reuse");
                crate::popup_window::prepare_window_reveal_by_title(&title)
            };
            if !prepared {
                return false;
            }
            let Some(fresh_frame) = schedule_fresh_frame(self.handle, bounds.size, cx) else {
                let _ = crate::popup_window::hide_invisible(&title);
                return false;
            };
            self.reveal_pending.set(true);
            qol_runtime::probe!(
                "SURFACE_REVEAL",
                "title={title} phase=opened hidden=true frame_scheduled=true reused=true x={} y={}",
                bounds.origin.x.to_f64(),
                bounds.origin.y.to_f64()
            );
            settle_then_reveal_reused(
                PendingReveal {
                    handle: self.handle,
                    title,
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

    pub(crate) fn resize(&mut self, size: Size<Pixels>, cx: &mut App) -> anyhow::Result<()> {
        let title = self.dismisser.current_title();
        if self.constrains_size && !constrain_native_size(&title, size) {
            return Err(anyhow!(
                "surface could not update its native size constraint"
            ));
        }
        self.handle.update(cx, |_, window, _| window.resize(size))?;
        self.size = size;
        Ok(())
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
            await_reveal_readiness(cx, handle, &title, origin, &fresh_frame).await;
        #[cfg(not(debug_assertions))]
        let _ = &attempts;
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
            "title={title} phase=frame-ready moved={} layout_confirmed={} viewport_ready={} fresh_frame={} content_rendered={} attempts={attempts} reused=true layout_epoch={}/{} render_epoch={}/{} gpu_completed_epoch={} expected={}x{} observed={}x{} rendered={}x{}",
            readiness.moved,
            readiness.layout_confirmed,
            readiness.viewport_ready,
            readiness.fresh_frame,
            readiness.content_rendered,
            fresh_frame.layout_epoch.get(),
            fresh_frame.required_layout_epoch,
            fresh_frame.render_epoch.get(),
            fresh_frame.required_render_epoch,
            fresh_frame.presented_render_epoch.get(),
            fresh_frame.expected_viewport.width.to_f64(),
            fresh_frame.expected_viewport.height.to_f64(),
            fresh_frame.observed_viewport.get().width.to_f64(),
            fresh_frame.observed_viewport.get().height.to_f64(),
            fresh_frame.rendered_viewport.get().width.to_f64(),
            fresh_frame.rendered_viewport.get().height.to_f64()
        );
        if !readiness.ready() {
            reveal_pending.set(false);
            let _reason = crate::popup_window::reason_scope("surface-reuse-timeout");
            let _ = crate::popup_window::hide_invisible(&title);
            qol_runtime::probe!(
                "SURFACE_REVEAL",
                "title={title} phase=revealed moved={} layout_confirmed={} viewport_ready={} fresh_frame={} content_rendered={} attempts={attempts} shown=false reused=true reason=frame-not-ready",
                readiness.moved,
                readiness.layout_confirmed,
                readiness.viewport_ready,
                readiness.fresh_frame,
                readiness.content_rendered
            );
            return;
        }
        let shown = {
            let _reason = crate::popup_window::reason_scope("surface-reuse-reveal");
            crate::popup_window::show_normal_window_by_title(&title)
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
        #[cfg(not(debug_assertions))]
        let _ = &repaint_requested;
        qol_runtime::probe!(
            "SURFACE_REVEAL",
            "title={title} phase=revealed moved={} layout_confirmed={} viewport_ready={} fresh_frame={} content_rendered={} attempts={attempts} shown=true reused=true repaint_requested={repaint_requested}",
            readiness.moved,
            readiness.layout_confirmed,
            readiness.viewport_ready,
            readiness.fresh_frame,
            readiness.content_rendered
        );
        let focus_commit = PANEL_FOCUS_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        crate::popup_window::reassert_normal_focus_until_held(
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
            #[cfg(not(debug_assertions))]
            let _ = attempt;
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

#[cfg(test)]
mod tests {
    use super::{viewport_matches, RevealReadiness, Surface, SurfaceKind};
    use crate::placement::MonitorPlacement;
    use gpui::{px, size, WindowKind};

    #[test]
    fn panel_surfaces_are_normal_focusable_windows() {
        let surface = Surface::new(SurfaceKind::Panel);

        assert_eq!(surface.window_kind(), WindowKind::Normal);
        assert_eq!(surface.placement, MonitorPlacement::center());
        assert!(surface.takes_focus());
        assert!(surface.constrains_size());
    }

    #[test]
    fn reveal_requires_placement_layout_viewport_content_and_a_completed_frame() {
        let cases = [
            (false, false, false, false, false, false),
            (true, false, true, true, true, false),
            (true, true, false, true, true, false),
            (true, true, true, false, true, false),
            (true, true, true, true, false, false),
            (true, true, true, true, true, true),
        ];
        for (moved, layout_confirmed, viewport_ready, fresh_frame, content_rendered, expected) in
            cases
        {
            assert_eq!(
                RevealReadiness {
                    moved,
                    layout_confirmed,
                    viewport_ready,
                    fresh_frame,
                    content_rendered,
                }
                .ready(),
                expected,
                "moved={moved} layout_confirmed={layout_confirmed} viewport_ready={viewport_ready} fresh_frame={fresh_frame} content_rendered={content_rendered}"
            );
        }
    }

    #[test]
    fn viewport_matching_accepts_native_rounding_only() {
        let expected = size(px(520.0), px(644.0));
        let cases = [
            ("exact", size(px(520.0), px(644.0)), true),
            ("one point taller", size(px(520.0), px(645.0)), true),
            ("one point narrower", size(px(519.0), px(644.0)), true),
            ("too tall", size(px(520.0), px(646.0)), false),
            ("too wide", size(px(522.0), px(644.0)), false),
        ];

        for (name, actual, matches) in cases {
            assert_eq!(viewport_matches(actual, expected), matches, "{name}");
        }
    }
}
