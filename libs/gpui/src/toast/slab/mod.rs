use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::*;

use super::card::{CardAct, CardHost};
use super::{pile, Activation, RowId, SlabSnapshotRow, Toast, ToastTone, WINDOW_ROOM};
use crate::monitor::{ActiveMonitor, MonitorTracker};
use crate::placement::MonitorPlacement;
use crate::popup_window::{present_topmost, restore_composite};
use crate::surface::{OpenedSurface, Surface, SurfaceKind};

mod view;

use view::SlabToastView;

const MAX_ROWS_PER_GROUP: usize = 3;
const HOVER_HOLD_RECHECK: Duration = Duration::from_millis(400);

struct ToastRow {
    id: RowId,
    toast: Rc<Toast>,
    generation: u64,
    created: Instant,
    deadline: Option<Instant>,
}

struct HostState {
    surface: Option<OpenedSurface<SlabToastView>>,
    rows: Vec<ToastRow>,
    next_id: u64,
    next_generation: u64,
    expanded: bool,
    hovering: bool,
    moving: bool,
    monitor: Option<Bounds<Pixels>>,
    watching: bool,
}

impl HostState {
    fn next_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.next_generation
    }
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
                rows: Vec::new(),
                next_id: 0,
                next_generation: 0,
                expanded: false,
                hovering: false,
                moving: false,
                monitor: None,
                watching: false,
            })),
            title: title.into(),
        }
    }

    pub(super) fn show(&self, toast: Toast, cx: &mut App) -> anyhow::Result<()> {
        let timeout = toast.effective_timeout();
        let (outcome, row_id, generation) = {
            let mut state = self.state.borrow_mut();
            let placement = toast.layout.placement();
            let group = toast.group.clone();
            let key = toast.key.clone();

            let target = key.as_ref().and_then(|key| {
                state
                    .rows
                    .iter()
                    .position(|row| row.toast.group == group && row.toast.key.as_ref() == Some(key))
            });

            let row_id;
            let generation;
            let created = Instant::now();
            let deadline = timeout.map(|timeout| created + timeout);
            let toast = Rc::new(toast);
            match target {
                Some(index) => {
                    generation = state.next_generation();
                    let row = &mut state.rows[index];
                    row.generation = generation;
                    row.toast = toast;
                    row.created = created;
                    row.deadline = deadline;
                    row_id = row.id;
                }
                None => {
                    row_id = RowId(state.next_id);
                    state.next_id += 1;
                    generation = state.next_generation();
                    state.rows.push(ToastRow {
                        id: row_id,
                        generation,
                        toast,
                        created,
                        deadline,
                    });
                }
            }

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
                state.rows.retain(|row| !stale_ids.contains(&row.id));
            }

            let outcome = if state.surface.is_none() && !state.moving {
                PushOutcome::Open { placement }
            } else {
                PushOutcome::Notify
            };
            (outcome, row_id, generation)
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

        if let Some(timeout) = timeout {
            arm_timer(self.clone(), row_id, generation, timeout, cx);
        }
        Ok(())
    }

    pub(super) fn dismiss(&self, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            state.rows.clear();
        }
        self.close(cx);
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
        let now = Instant::now();
        let armed: Vec<(RowId, u64, Duration)> = {
            let mut state = self.state.borrow_mut();
            let mut armed = Vec::new();
            for index in 0..state.rows.len() {
                let Some(timeout) = state.rows[index].toast.effective_timeout() else {
                    continue;
                };
                let generation = state.next_generation();
                let row = &mut state.rows[index];
                row.generation = generation;
                row.deadline = Some(now + timeout);
                armed.push((row.id, generation, timeout));
            }
            armed
        };
        for (id, generation, timeout) in armed {
            arm_timer(self.clone(), id, generation, timeout, cx);
        }
    }

    fn mark_row_failed(&self, id: RowId, error: anyhow::Error, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            let generation = state.next_generation();
            if let Some(row) = state.rows.iter_mut().find(|row| row.id == id) {
                row.generation = generation;
                let toast = Rc::make_mut(&mut row.toast);
                let previous_title = toast.title.clone();
                toast.tone = ToastTone::Danger;
                toast.message = previous_title;
                toast.title = error.to_string().into();
                toast.timeout = None;
                toast.timeout_explicit = true;
                row.deadline = None;
            }
        }
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

    fn on_timer(&self, id: RowId, generation: u64, cx: &mut App) {
        let (owned, held) = {
            let state = self.state.borrow();
            let owned = state
                .rows
                .iter()
                .any(|row| row.id == id && row.generation == generation);
            (owned, state.hovering || state.expanded)
        };
        if !owned {
            return;
        }
        if held {
            arm_timer(self.clone(), id, generation, HOVER_HOLD_RECHECK, cx);
        } else {
            self.remove(id, cx);
        }
    }

    fn remove(&self, id: RowId, cx: &mut App) {
        self.drop_rows(&[id], cx);
    }

    fn drop_rows(&self, ids: &[RowId], cx: &mut App) {
        let remains_empty = {
            let mut state = self.state.borrow_mut();
            state.rows.retain(|row| !ids.contains(&row.id));
            if state.rows.len() < 2 {
                state.expanded = false;
            }
            state.rows.is_empty()
        };
        if remains_empty && self.state.borrow().surface.is_none() {
            self.close(cx);
        } else {
            self.notify_view(cx);
        }
    }

    fn watch_monitor(&self, cx: &mut App) {
        if std::mem::replace(&mut self.state.borrow_mut().watching, true) {
            return;
        }
        let presenter = self.clone();
        crate::event_router::spawn_runtime_event_router(
            cx,
            vec![crate::protocol::RuntimeEventKind::ActiveMonitorChanged],
            move |cx, event| {
                let monitor = ActiveMonitor::from_event(event).map(|monitor| monitor.bounds());
                presenter.follow(monitor, cx);
            },
        );
    }

    fn open_surface(
        &self,
        placement: MonitorPlacement,
        scroll: f32,
        arriving: bool,
        cx: &mut App,
    ) -> Result<()> {
        self.state.borrow_mut().moving = false;
        let monitor = self
            .tracker
            .snapshot_monitor()
            .or_else(|| self.tracker.snapshot_cursor().map(|(monitor, _)| monitor));
        let host = self.clone();
        let surface = Surface::new(SurfaceKind::Toast)
            .title(&*self.title)
            .placement(placement)
            .size(size(px(pile::WIDTH.ceil()), px(WINDOW_ROOM)))
            .open_on(monitor.as_ref(), cx, move |dismisser, _window, _cx| {
                SlabToastView::new(host, dismisser, scroll, arriving)
            })?;
        let replaced = {
            let mut state = self.state.borrow_mut();
            state.monitor = monitor.map(|monitor| monitor.bounds());
            state.surface.replace(surface)
        };
        if let Some(replaced) = replaced {
            replaced.dismisser.dismiss(cx);
        }
        present_topmost(&self.title);
        self.watch_monitor(cx);
        Ok(())
    }

    fn follow(&self, monitor: Option<Bounds<Pixels>>, cx: &mut App) {
        let (placement, handle) = {
            let state = self.state.borrow();
            let Some(surface) = state.surface.as_ref() else {
                return;
            };
            if monitor.is_none() || state.monitor == monitor {
                return;
            }
            (surface.placement(), surface.handle)
        };
        let scroll = handle
            .update(cx, |root, _, cx| {
                root.inner.update(cx, |view, _| view.moving_away())
            })
            .unwrap_or(0.0);
        let surface = {
            let mut state = self.state.borrow_mut();
            state.moving = true;
            state.hovering = false;
            state.surface.take()
        };
        if let Some(surface) = surface {
            surface.dismisser.dismiss(cx);
        }
        restore_composite(&self.title);
        let presenter = self.clone();
        cx.defer(move |cx| {
            if presenter.state.borrow().rows.is_empty() {
                presenter.state.borrow_mut().moving = false;
                return;
            }
            if let Err(error) = presenter.open_surface(placement, scroll, true, cx) {
                log::warn!("[toast] the stack could not move to the active monitor: {error:#}");
            }
        });
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
        let rows = state
            .rows
            .iter()
            .map(|row| SlabSnapshotRow {
                id: row.id,
                toast: row.toast.clone(),
                created: row.created,
                deadline: row.deadline,
            })
            .collect();
        (rows, state.expanded)
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

fn arm_timer(
    presenter: SlabPresenter,
    id: RowId,
    generation: u64,
    timeout: Duration,
    cx: &mut App,
) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(timeout).await;
        let _ = cx.update(|cx| presenter.on_timer(id, generation, cx));
    })
    .detach();
}
