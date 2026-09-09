use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::*;

use super::platform::{Platform, SurfacePlatform};

const REVEAL_POLL_INTERVAL: Duration = Duration::from_millis(15);
const REVEAL_MAX_ATTEMPTS: usize = 40;
const VIEWPORT_TOLERANCE: f64 = 1.0;

#[derive(Clone, Copy, Debug)]
pub struct RevealProof {
    pub layout_confirmed: bool,
    pub viewport_ready: bool,
    pub fresh_frame: bool,
    pub content_rendered: bool,
    pub layout_epoch: u64,
    pub required_layout_epoch: u64,
    pub render_epoch: u64,
    pub required_render_epoch: u64,
    pub presented_epoch: u64,
    pub expected_viewport: Size<Pixels>,
    pub observed_viewport: Size<Pixels>,
    pub rendered_viewport: Size<Pixels>,
}

impl RevealProof {
    pub fn ready(&self) -> bool {
        self.layout_confirmed && self.viewport_ready && self.fresh_frame && self.content_rendered
    }
}

pub struct RevealOutcome {
    pub proof: RevealProof,
    pub moved: bool,
    pub attempts: usize,
    pub cancelled: bool,
    session: Option<crate::popup_window::WindowGeometrySession>,
}

impl RevealOutcome {
    pub fn session_connected(&self) -> bool {
        self.session.is_some()
    }

    pub fn reposition(&self, title: &str, origin: Point<Pixels>) -> bool {
        reposition_reveal_window(self.session.as_ref(), title, origin)
    }
}

#[derive(Clone)]
struct FrameCells {
    layout_epoch: Rc<Cell<u64>>,
    observed_viewport: Rc<Cell<Size<Pixels>>>,
    rendered_layout_epoch: Rc<Cell<u64>>,
    rendered_viewport: Rc<Cell<Size<Pixels>>>,
    render_epoch: Rc<Cell<u64>>,
    presented_render_epoch: Rc<Cell<u64>>,
    presented_layout_epoch: Rc<Cell<u64>>,
}

pub struct FreshFrame {
    cells: FrameCells,
    expected_viewport: Rc<Cell<Size<Pixels>>>,
    required_layout_epoch: u64,
    required_render_epoch: u64,
    request_frame: Box<dyn Fn(&mut App) -> bool>,
    _bounds_subscription: Subscription,
}

impl FreshFrame {
    fn request(&self, cx: &mut App) -> bool {
        (self.request_frame)(cx)
    }

    fn snapshot(&self) -> RevealProof {
        RevealProof {
            layout_confirmed: self.layout_confirmed(),
            viewport_ready: self.viewport_ready(),
            fresh_frame: self.presented(),
            content_rendered: self.content_rendered(),
            layout_epoch: self.cells.layout_epoch.get(),
            required_layout_epoch: self.required_layout_epoch,
            render_epoch: self.cells.render_epoch.get(),
            required_render_epoch: self.required_render_epoch,
            presented_epoch: self.cells.presented_render_epoch.get(),
            expected_viewport: self.expected_viewport.get(),
            observed_viewport: self.cells.observed_viewport.get(),
            rendered_viewport: self.cells.rendered_viewport.get(),
        }
    }

    fn layout_confirmed(&self) -> bool {
        Platform::layout_confirmed(
            self.cells.layout_epoch.get(),
            self.required_layout_epoch,
            self.cells.observed_viewport.get(),
            self.expected_viewport.get(),
            VIEWPORT_TOLERANCE,
        )
    }

    fn viewport_ready(&self) -> bool {
        Platform::viewport_matches(
            self.cells.rendered_viewport.get(),
            self.expected_viewport.get(),
            VIEWPORT_TOLERANCE,
        )
    }

    fn presented(&self) -> bool {
        self.cells.presented_render_epoch.get() >= self.required_render_epoch
            && self.cells.presented_layout_epoch.get() >= self.required_layout_epoch
    }

    fn content_rendered(&self) -> bool {
        self.cells.render_epoch.get() >= self.required_render_epoch
            && self.cells.rendered_layout_epoch.get() >= self.required_layout_epoch
            && self.viewport_ready()
    }
}

pub fn schedule_fresh_frame<V: Render + 'static>(
    handle: WindowHandle<V>,
    expected_viewport: Rc<Cell<Size<Pixels>>>,
    cx: &mut App,
) -> Option<FreshFrame> {
    handle
        .update(cx, |_, window, cx| {
            schedule_fresh_frame_in(window, cx, expected_viewport)
        })
        .ok()
        .flatten()
}

pub fn schedule_fresh_frame_in<V: Render + 'static>(
    window: &mut Window,
    cx: &mut Context<V>,
    expected_viewport: Rc<Cell<Size<Pixels>>>,
) -> Option<FreshFrame> {
    let handle = window.window_handle().downcast::<V>()?;
    let cells = FrameCells {
        layout_epoch: Rc::new(Cell::new(0)),
        observed_viewport: Rc::new(Cell::new(size(px(0.0), px(0.0)))),
        rendered_layout_epoch: Rc::new(Cell::new(0)),
        rendered_viewport: Rc::new(Cell::new(size(px(0.0), px(0.0)))),
        render_epoch: Rc::new(Cell::new(0)),
        presented_render_epoch: Rc::new(Cell::new(0)),
        presented_layout_epoch: Rc::new(Cell::new(0)),
    };
    let observed = cells.clone();
    let bounds_subscription = cx.observe_window_bounds(window, move |_, window, _| {
        observed
            .layout_epoch
            .set(observed.layout_epoch.get().wrapping_add(1));
        observed.observed_viewport.set(window.viewport_size());
    });
    let request_cells = cells.clone();
    let fresh_frame = FreshFrame {
        required_layout_epoch: Platform::required_layout_epoch(cells.layout_epoch.get()),
        required_render_epoch: cells.render_epoch.get().wrapping_add(1),
        cells,
        expected_viewport,
        request_frame: Box::new(move |cx: &mut App| {
            handle
                .update(cx, |_, window, cx| {
                    request_frame_in(window, cx, &request_cells)
                })
                .is_ok()
        }),
        _bounds_subscription: bounds_subscription,
    };
    request_frame_in(window, cx, &fresh_frame.cells);
    Some(fresh_frame)
}

pub async fn await_reveal_readiness(
    cx: &mut AsyncApp,
    title: &str,
    fresh_frame: &FreshFrame,
    is_cancelled: impl Fn(&AsyncApp) -> bool,
    target_origin: impl Fn() -> Option<Point<Pixels>>,
) -> RevealOutcome {
    let mut moved = target_origin().is_none();
    let mut geometry_session = None;
    let mut proof = fresh_frame.snapshot();
    for attempt in 1..=REVEAL_MAX_ATTEMPTS {
        if is_cancelled(cx) {
            return RevealOutcome {
                proof,
                moved,
                attempts: attempt - 1,
                cancelled: true,
                session: geometry_session,
            };
        }
        cx.background_executor().timer(REVEAL_POLL_INTERVAL).await;
        if is_cancelled(cx) {
            return RevealOutcome {
                proof,
                moved,
                attempts: attempt,
                cancelled: true,
                session: geometry_session,
            };
        }
        let origin = target_origin();
        if geometry_session.is_none() && origin.is_some() {
            let session_title = title.to_owned();
            geometry_session = cx
                .background_spawn(async move {
                    crate::popup_window::window_geometry_session(&session_title)
                })
                .await;
            qol_runtime::probe!(
                "SURFACE_REVEAL",
                "title={title} phase=geometry-session attempt={attempt} connected={}",
                geometry_session.is_some()
            );
            if is_cancelled(cx) {
                return RevealOutcome {
                    proof,
                    moved,
                    attempts: attempt,
                    cancelled: true,
                    session: geometry_session,
                };
            }
        }
        if let Some(origin) = origin {
            moved |= reposition_reveal_window(geometry_session.as_ref(), title, origin);
        }
        proof = fresh_frame.snapshot();
        if proof.ready() {
            return RevealOutcome {
                proof,
                moved,
                attempts: attempt,
                cancelled: false,
                session: geometry_session,
            };
        }
        let frame_requested = cx.update(|cx| fresh_frame.request(cx)).unwrap_or(false);
        if !frame_requested {
            return RevealOutcome {
                proof,
                moved,
                attempts: attempt,
                cancelled: false,
                session: geometry_session,
            };
        }
    }
    RevealOutcome {
        proof,
        moved,
        attempts: REVEAL_MAX_ATTEMPTS,
        cancelled: false,
        session: geometry_session,
    }
}

fn request_frame_in<V: Render + 'static>(
    window: &mut Window,
    cx: &mut Context<V>,
    cells: &FrameCells,
) {
    let frame_cells = cells.clone();
    window.on_next_frame(move |window, _cx| {
        let epoch = frame_cells.layout_epoch.get();
        let viewport = window.viewport_size();
        let rendered = frame_cells.clone();
        window.on_next_frame(move |_window, _cx| {
            let completed = rendered.render_epoch.get().wrapping_add(1);
            rendered.render_epoch.set(completed);
            rendered.rendered_layout_epoch.set(epoch);
            rendered.rendered_viewport.set(viewport);
            rendered.presented_render_epoch.set(completed);
            rendered.presented_layout_epoch.set(epoch);
        });
    });
    cx.notify();
    window.refresh();
}

fn reposition_reveal_window(
    geometry_session: Option<&crate::popup_window::WindowGeometrySession>,
    title: &str,
    origin: Point<Pixels>,
) -> bool {
    geometry_session.map_or_else(
        || {
            crate::popup_window::reposition_window_by_title(
                title,
                origin.x.to_f64(),
                origin.y.to_f64(),
            )
        },
        |session| session.reposition(origin.x.to_f64() as i32, origin.y.to_f64() as i32),
    )
}
