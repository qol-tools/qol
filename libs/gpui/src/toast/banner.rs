use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::*;

use super::card::{self, CardAct, CardHost};
use super::{Activation, RowId, SlabSnapshotRow, Tick, Toast, ToastLayout, AGE_TICK, RING_TICK};
use crate::monitor::MonitorTracker;
use crate::surface::{OpenedSurface, Surface, SurfaceDismisser, SurfaceKind};

struct ActiveToast {
    surface: OpenedSurface<BannerToastView>,
    layout: ToastLayout,
}

#[derive(Clone)]
pub(super) struct BannerPresenter {
    tracker: MonitorTracker,
    active: Rc<RefCell<Option<ActiveToast>>>,
    generation: Rc<Cell<u64>>,
    pub(super) title: Rc<str>,
}

impl BannerPresenter {
    pub(super) fn new(tracker: MonitorTracker, title: impl Into<Rc<str>>) -> Self {
        Self {
            tracker,
            active: Rc::new(RefCell::new(None)),
            generation: Rc::new(Cell::new(0)),
            title: title.into(),
        }
    }

    pub(super) fn show(&self, toast: Toast, cx: &mut App) -> anyhow::Result<()> {
        let timeout = toast.effective_timeout();
        if !self.update_active(toast.clone(), cx) {
            self.close_active(cx);
            let layout = toast.layout;
            let surface = self.open(toast, cx)?;
            self.active
                .borrow_mut()
                .replace(ActiveToast { surface, layout });
        }
        let generation = self.next_generation();
        if let Some(timeout) = timeout {
            self.dismiss_after(generation, timeout, cx);
        }
        Ok(())
    }

    pub(super) fn dismiss(&self, cx: &mut App) {
        self.next_generation();
        self.close_active(cx);
    }

    fn open(&self, toast: Toast, cx: &mut App) -> anyhow::Result<OpenedSurface<BannerToastView>> {
        let host = self.clone();
        Surface::new(SurfaceKind::Toast)
            .title(&*self.title)
            .placement(toast.layout.placement())
            .size(toast.layout.size())
            .open(&self.tracker, cx, move |dismisser, _window, _cx| {
                BannerToastView::new(toast, host, dismisser)
            })
    }

    fn update_active(&self, toast: Toast, cx: &mut App) -> bool {
        let mut slot = self.active.borrow_mut();
        let Some(active) = slot.as_mut() else {
            return false;
        };
        if active.layout != toast.layout {
            return false;
        }
        let updated = active
            .surface
            .handle
            .update(cx, |root, _, cx| {
                root.inner.update(cx, |view, cx| {
                    view.show(Rc::new(toast));
                    cx.notify();
                });
            })
            .is_ok();
        if !updated {
            *slot = None;
        }
        updated
    }

    fn run(&self, action: Option<Activation>, cx: &mut App) {
        let generation = self.generation.get();
        if let Some(action) = action {
            if let Err(error) = action(cx) {
                qol_runtime::probe!("TOAST_ACTIVATION", "presentation=banner error={error:#}");
            }
        }
        if self.generation.get() == generation {
            self.dismiss(cx);
        }
    }

    fn close_active(&self, cx: &mut App) {
        let Some(active) = self.active.borrow_mut().take() else {
            return;
        };
        active.surface.dismisser.dismiss(cx);
    }

    fn next_generation(&self) -> u64 {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        generation
    }

    fn dismiss_after(&self, generation: u64, timeout: Duration, cx: &mut App) {
        let presenter = self.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(timeout).await;
            if presenter.generation.get() != generation {
                return;
            }
            let _ = cx.update(|cx| presenter.dismiss(cx));
        })
        .detach();
    }
}

struct BannerCard {
    presenter: BannerPresenter,
    toast: Rc<Toast>,
}

impl CardHost for BannerCard {
    fn act(&self, _id: RowId, act: CardAct, cx: &mut App) {
        match act {
            CardAct::Open => self.presenter.run(self.toast.activation.clone(), cx),
            CardAct::Preview => self.presenter.run(self.toast.preview_action.clone(), cx),
            CardAct::Close => self.presenter.dismiss(cx),
        }
    }
}

struct BannerToastView {
    row: SlabSnapshotRow,
    host: BannerPresenter,
    card: card::Host,
    ring: Tick,
    age: Tick,
    _dismisser: SurfaceDismisser,
}

impl BannerToastView {
    fn new(toast: Toast, host: BannerPresenter, dismisser: SurfaceDismisser) -> Self {
        let toast = Rc::new(toast);
        let mut view = Self {
            row: SlabSnapshotRow {
                id: RowId(0),
                toast: toast.clone(),
                created: Instant::now(),
                deadline: None,
            },
            card: Rc::new(BannerCard {
                presenter: host.clone(),
                toast: toast.clone(),
            }),
            host,
            ring: Tick::default(),
            age: Tick::default(),
            _dismisser: dismisser,
        };
        view.show(toast);
        view
    }

    fn show(&mut self, toast: Rc<Toast>) {
        let now = Instant::now();
        self.row.deadline = toast.effective_timeout().map(|timeout| now + timeout);
        self.row.created = now;
        self.card = Rc::new(BannerCard {
            presenter: self.host.clone(),
            toast: toast.clone(),
        });
        self.row.toast = toast;
    }
}

impl Render for BannerToastView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if self.row.deadline.is_some_and(|deadline| deadline > now) {
            self.ring.after(RING_TICK, cx);
        } else {
            self.age.after(AGE_TICK, cx);
        }
        card::lone(&self.row, Some(self.card.clone()), now)
    }
}
