use std::cell::{RefCell, RefMut};
use std::rc::Rc;
use std::time::Instant;

use gpui::*;

use super::card::{CardAct, CardHost};
use super::follow::{self, Follow, Follower};
use super::rows::{self, Rows, Timed};
use super::{pile, Activation, RowId, SlabSnapshotRow, Toast, ToastTone, WINDOW_ROOM};
use crate::monitor::MonitorTracker;
use crate::placement::MonitorPlacement;
use crate::popup_window::{present_topmost, restore_composite};
use crate::surface::{OpenedSurface, Surface, SurfaceKind};

mod view;

use view::SlabToastView;

const MAX_ROWS_PER_GROUP: usize = 3;

struct HostState {
    surface: Option<OpenedSurface<SlabToastView>>,
    rows: Rows,
    expanded: bool,
    hovering: bool,
    follow: Follow,
}

#[derive(Clone)]
pub(super) struct SlabPresenter {
    tracker: MonitorTracker,
    state: Rc<RefCell<HostState>>,
    pub(super) title: Rc<str>,
}

enum PushOutcome {
    Open { placement: MonitorPlacement },
    Notify,
}

impl SlabPresenter {
    pub(super) fn new(tracker: MonitorTracker, title: impl Into<Rc<str>>) -> Self {
        Self {
            tracker,
            state: Rc::new(RefCell::new(HostState {
                surface: None,
                rows: Rows::default(),
                expanded: false,
                hovering: false,
                follow: Follow::default(),
            })),
            title: title.into(),
        }
    }

    pub(super) fn show(&self, toast: Toast, cx: &mut App) -> anyhow::Result<()> {
        let (outcome, timer) = {
            let mut state = self.state.borrow_mut();
            let placement = toast.layout.placement();
            let group = toast.group.clone();
            state.rows.put(toast, rows::same_key);

            let positions: Vec<usize> = state
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| !row.toast.live && row.toast.group == group)
                .map(|(index, _)| index)
                .collect();
            if positions.len() > MAX_ROWS_PER_GROUP {
                let stale_ids: Vec<RowId> = positions[..positions.len() - MAX_ROWS_PER_GROUP]
                    .iter()
                    .map(|&index| state.rows[index].id)
                    .collect();
                state.rows.remove(&stale_ids);
            }

            let outcome = if state.surface.is_none() && !state.follow.moving() {
                PushOutcome::Open { placement }
            } else {
                PushOutcome::Notify
            };
            (outcome, state.rows.time_front(Instant::now()))
        };

        match outcome {
            PushOutcome::Open { placement } => {
                if let Err(error) = self.open_surface(placement, 0.0, false, cx) {
                    let mut state = self.state.borrow_mut();
                    state.rows.clear();
                    state.expanded = false;
                    return Err(error);
                }
            }
            PushOutcome::Notify => self.notify_view(cx),
        }

        rows::arm(self, timer, cx);
        Ok(())
    }

    pub(super) fn dismiss(&self, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            state.rows.clear();
        }
        self.close(cx);
    }

    pub(super) fn withdraw(&self, toast: &Toast, cx: &mut App) {
        let ids = self.state.borrow().rows.keyed_like(toast);
        if !ids.is_empty() {
            self.drop_rows(&ids, cx);
        }
    }

    fn clear_all(&self, cx: &mut App) {
        let ids: Vec<RowId> = {
            let state = self.state.borrow();
            state
                .rows
                .iter()
                .filter(|row| !row.toast.live)
                .map(|row| row.id)
                .collect()
        };
        self.drop_rows(&ids, cx);
    }

    fn set_expanded(&self, expanded: bool, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            let expanded = expanded && state.rows.len() >= 2;
            if state.expanded == expanded {
                return;
            }
            state.expanded = expanded;
        }
        if !expanded {
            self.restart_timers(cx);
        }
        self.notify_view(cx);
    }

    fn set_hovering(&self, hovering: bool) {
        self.state.borrow_mut().hovering = hovering;
    }

    fn restart_timers(&self, cx: &mut App) {
        let timer = self.state.borrow_mut().rows.restart_front(Instant::now());
        rows::arm(self, timer, cx);
    }

    fn mark_row_failed(&self, id: RowId, error: anyhow::Error, cx: &mut App) {
        let timer = {
            let mut state = self.state.borrow_mut();
            let generation = state.rows.next_generation();
            if let Some(row) = state.rows.iter_mut().find(|row| row.id == id) {
                row.generation = generation;
                let toast = Rc::make_mut(&mut row.toast);
                let previous_title = toast.title.clone();
                toast.tone = ToastTone::Danger;
                toast.message = previous_title;
                toast.title = error.to_string().into();
                toast.timeout = None;
                toast.timeout_explicit = false;
                row.deadline = None;
            }
            state.rows.time_front(Instant::now())
        };
        rows::arm(self, timer, cx);
        self.notify_view(cx);
    }

    fn run(&self, id: RowId, pick: impl Fn(&Toast) -> Option<Activation>, cx: &mut App) {
        let action = {
            let state = self.state.borrow();
            state
                .rows
                .iter()
                .find(|row| row.id == id)
                .and_then(|row| pick(&row.toast))
        };
        let Some(action) = action else {
            return;
        };
        match action(cx) {
            Ok(()) => self.remove(id, cx),
            Err(error) => self.mark_row_failed(id, error, cx),
        }
    }

    fn remove(&self, id: RowId, cx: &mut App) {
        self.drop_rows(&[id], cx);
    }

    fn drop_rows(&self, ids: &[RowId], cx: &mut App) {
        let (remains_empty, timer) = {
            let mut state = self.state.borrow_mut();
            state.rows.remove(ids);
            if state.rows.len() < 2 {
                state.expanded = false;
            }
            (state.rows.is_empty(), state.rows.time_front(Instant::now()))
        };
        rows::arm(self, timer, cx);
        if remains_empty && self.state.borrow().surface.is_none() {
            self.close(cx);
        } else {
            self.notify_view(cx);
        }
    }

    fn open_surface(
        &self,
        placement: MonitorPlacement,
        scroll: f32,
        arriving: bool,
        cx: &mut App,
    ) -> Result<()> {
        let monitor = self.state.borrow_mut().follow.land(&self.tracker);
        let host = self.clone();
        let surface = Surface::new(SurfaceKind::Toast)
            .title(&*self.title)
            .placement(placement)
            .size(size(px(pile::WIDTH.ceil()), px(WINDOW_ROOM)))
            .open_on(monitor.as_ref(), cx, move |dismisser, _window, _cx| {
                SlabToastView::new(host, dismisser, scroll, arriving)
            })?;
        let replaced = self.state.borrow_mut().surface.replace(surface);
        if let Some(replaced) = replaced {
            replaced.dismisser.dismiss(cx);
        }
        present_topmost(&self.title);
        follow::watch(self, cx);
        Ok(())
    }

    fn close_if_empty(&self, cx: &mut App) {
        if self.state.borrow().rows.is_empty() {
            self.close(cx);
        }
    }

    fn close(&self, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            state.expanded = false;
            state.hovering = false;
        }
        if let Some(surface) = self.state.borrow_mut().surface.take() {
            surface.dismisser.dismiss(cx);
        }
        restore_composite(&self.title);
    }

    fn notify_view(&self, cx: &mut App) {
        if let Some(surface) = self.state.borrow().surface.as_ref() {
            let _ = surface.handle.update(cx, |root, _, cx| {
                root.inner.update(cx, |_view, cx| cx.notify());
            });
        }
    }

    fn slab_snapshot(&self) -> (Vec<SlabSnapshotRow>, bool) {
        let state = self.state.borrow();
        (state.rows.snapshot(), state.expanded)
    }
}

impl Timed for SlabPresenter {
    fn rows(&self) -> RefMut<'_, Rows> {
        RefMut::map(self.state.borrow_mut(), |state| &mut state.rows)
    }

    fn held(&self) -> bool {
        let state = self.state.borrow();
        state.hovering || state.expanded
    }

    fn expire(&self, id: RowId, cx: &mut App) {
        self.remove(id, cx);
    }
}

impl Follower for SlabPresenter {
    type Carry = (MonitorPlacement, f32);

    fn follow(&self) -> RefMut<'_, Follow> {
        RefMut::map(self.state.borrow_mut(), |state| &mut state.follow)
    }

    fn depart(&self, cx: &mut App) -> Option<Self::Carry> {
        let (placement, handle) = {
            let state = self.state.borrow();
            let surface = state.surface.as_ref()?;
            (surface.placement(), surface.handle)
        };
        let scroll = handle
            .update(cx, |root, _, cx| {
                root.inner.update(cx, |view, _| view.moving_away())
            })
            .unwrap_or(0.0);
        let surface = {
            let mut state = self.state.borrow_mut();
            state.hovering = false;
            state.surface.take()
        };
        if let Some(surface) = surface {
            surface.dismisser.dismiss(cx);
        }
        restore_composite(&self.title);
        Some((placement, scroll))
    }

    fn empty(&self) -> bool {
        self.state.borrow().rows.is_empty()
    }

    fn arrive(&self, (placement, scroll): Self::Carry, cx: &mut App) -> anyhow::Result<()> {
        self.open_surface(placement, scroll, true, cx)
    }
}

impl CardHost for SlabPresenter {
    fn act(&self, id: RowId, act: CardAct, cx: &mut App) {
        match act {
            CardAct::Open => self.run(id, |toast| toast.activation.clone(), cx),
            CardAct::Preview => self.run(id, |toast| toast.preview_action.clone(), cx),
            CardAct::Close => self.remove(id, cx),
        }
    }
}
