use std::cell::{RefCell, RefMut};
use std::rc::Rc;
use std::time::Instant;

use gpui::*;
use qol_theme::Motion;

use super::card::{self, CardAct, CardHost};
use super::follow::{self, Follow, Follower};
use super::pile::{self, Frame, EDGE_INSET, EDGE_STEP, MAX_EDGES};
use super::rows::{self, Rows, Timed};
use super::shape::InputShape;
use super::{Activation, RowId, SlabSnapshotRow, Toast, ToastLayout};
use crate::monitor::MonitorTracker;
use crate::popup_window::InputEvent;
use crate::surface::{OpenedSurface, Surface, SurfaceDismisser, SurfaceKind};

const MOST: usize = MAX_EDGES + 1;

struct BannerState {
    surface: Option<OpenedSurface<BannerToastView>>,
    layout: Option<ToastLayout>,
    rows: Rows,
    emptied: u64,
    follow: Follow,
    hovering: bool,
}

#[derive(Clone)]
pub(super) struct BannerPresenter {
    tracker: MonitorTracker,
    state: Rc<RefCell<BannerState>>,
    pub(super) title: Rc<str>,
}

impl BannerPresenter {
    pub(super) fn new(tracker: MonitorTracker, title: impl Into<Rc<str>>) -> Self {
        Self {
            tracker,
            state: Rc::new(RefCell::new(BannerState {
                surface: None,
                layout: None,
                rows: Rows::default(),
                emptied: 0,
                follow: Follow::default(),
                hovering: false,
            })),
            title: title.into(),
        }
    }

    pub(super) fn show(&self, toast: Toast, cx: &mut App) -> anyhow::Result<()> {
        let toast = toast.for_the_top();
        self.forget_lost_window(cx);
        let layout = toast.layout;
        if self
            .state
            .borrow()
            .layout
            .is_some_and(|open| open != layout)
        {
            self.close(cx);
        }
        let (timer, closed) = {
            let mut state = self.state.borrow_mut();
            let (id, _) = state.rows.put(toast, same_message);
            state.rows.raise(id);
            state.rows.keep_newest(MOST);
            (
                state.rows.time_front(Instant::now()),
                state.surface.is_none() && !state.follow.moving(),
            )
        };
        if closed {
            if let Err(error) = self.open(layout, cx) {
                self.state.borrow_mut().rows.clear();
                return Err(error);
            }
        } else {
            self.notify(cx);
        }
        rows::arm(self, timer, cx);
        Ok(())
    }

    pub(super) fn dismiss(&self, cx: &mut App) {
        self.state.borrow_mut().rows.clear();
        self.changed(cx);
    }

    pub(super) fn withdraw(&self, toast: &Toast, cx: &mut App) {
        let ids = self.state.borrow().rows.keyed_like(toast);
        if !ids.is_empty() {
            self.state.borrow_mut().rows.remove(&ids);
            self.changed(cx);
        }
    }

    fn forget_lost_window(&self, cx: &App) {
        let mut state = self.state.borrow_mut();
        let lost = state.surface.as_ref().is_some_and(|surface| {
            let id = surface.handle.window_id();
            cx.windows().iter().all(|window| window.window_id() != id)
        });
        if lost {
            state.surface = None;
            state.layout = None;
        }
    }

    fn open(&self, layout: ToastLayout, cx: &mut App) -> anyhow::Result<()> {
        let monitor = self.state.borrow_mut().follow.land(&self.tracker);
        let host = self.clone();
        let card = layout.size();
        let surface = Surface::new(SurfaceKind::Toast)
            .title(&*self.title)
            .placement(layout.placement())
            .size(size(
                card.width,
                card.height + px(EDGE_STEP * MAX_EDGES as f32),
            ))
            .open_on(monitor.as_ref(), cx, move |dismisser, _window, _cx| {
                BannerToastView::new(host, dismisser, card)
            })?;
        let replaced = {
            let mut state = self.state.borrow_mut();
            state.layout = Some(layout);
            state.surface.replace(surface)
        };
        if let Some(replaced) = replaced {
            replaced.dismisser.dismiss(cx);
        }
        if self.state.borrow().hovering {
            self.set_hovering(false, cx);
        }
        follow::watch(self, cx);
        Ok(())
    }

    fn set_hovering(&self, hovering: bool, cx: &mut App) {
        let timer = {
            let mut state = self.state.borrow_mut();
            state.hovering = hovering;
            if hovering {
                None
            } else {
                state.rows.restart_front(Instant::now())
            }
        };
        rows::arm(self, timer, cx);
    }

    fn remove(&self, id: RowId, cx: &mut App) {
        self.state.borrow_mut().rows.remove(&[id]);
        self.changed(cx);
    }

    fn changed(&self, cx: &mut App) {
        let timer = self.state.borrow_mut().rows.time_front(Instant::now());
        rows::arm(self, timer, cx);
        let emptied = {
            let mut state = self.state.borrow_mut();
            if !state.rows.is_empty() || state.surface.is_none() {
                None
            } else {
                state.emptied += 1;
                Some(state.emptied)
            }
        };
        self.notify(cx);
        if let Some(emptied) = emptied {
            let presenter = self.clone();
            rows::after(Motion::LEAVE.duration, cx, move |cx| {
                let still = {
                    let state = presenter.state.borrow();
                    state.emptied == emptied && state.rows.is_empty()
                };
                if still {
                    presenter.close(cx);
                }
            });
        }
    }

    fn run(&self, id: RowId, pick: impl Fn(&Toast) -> Option<Activation>, cx: &mut App) {
        let found = {
            let state = self.state.borrow();
            state
                .rows
                .iter()
                .find(|row| row.id == id)
                .map(|row| (row.toast.clone(), pick(&row.toast)))
        };
        let Some((clicked, action)) = found else {
            return;
        };
        if let Some(action) = action {
            if let Err(error) = action(cx) {
                qol_runtime::probe!("TOAST_ACTIVATION", "presentation=banner error={error:#}");
            }
        }
        let unchanged = self
            .state
            .borrow()
            .rows
            .iter()
            .any(|row| row.id == id && Rc::ptr_eq(&row.toast, &clicked));
        if unchanged {
            self.remove(id, cx);
        }
    }

    fn close(&self, cx: &mut App) {
        let surface = {
            let mut state = self.state.borrow_mut();
            state.rows.clear();
            state.layout = None;
            state.surface.take()
        };
        if let Some(surface) = surface {
            surface.dismisser.dismiss(cx);
        }
    }

    fn notify(&self, cx: &mut App) {
        if let Some(surface) = self.state.borrow().surface.as_ref() {
            let _ = surface.handle.update(cx, |root, _, cx| {
                root.inner.update(cx, |_view, cx| cx.notify());
            });
        }
    }

    fn snapshot(&self) -> Vec<SlabSnapshotRow> {
        self.state.borrow().rows.snapshot()
    }
}

impl Timed for BannerPresenter {
    fn rows(&self) -> RefMut<'_, Rows> {
        RefMut::map(self.state.borrow_mut(), |state| &mut state.rows)
    }

    fn held(&self) -> bool {
        self.state.borrow().hovering
    }

    fn expire(&self, id: RowId, cx: &mut App) {
        self.remove(id, cx);
    }
}

impl Follower for BannerPresenter {
    type Carry = ToastLayout;

    fn follow(&self) -> RefMut<'_, Follow> {
        RefMut::map(self.state.borrow_mut(), |state| &mut state.follow)
    }

    fn depart(&self, cx: &mut App) -> Option<ToastLayout> {
        let (surface, layout) = {
            let mut state = self.state.borrow_mut();
            let layout = state.layout?;
            (state.surface.take()?, layout)
        };
        surface.dismisser.dismiss(cx);
        Some(layout)
    }

    fn empty(&self) -> bool {
        self.state.borrow().rows.is_empty()
    }

    fn arrive(&self, layout: ToastLayout, cx: &mut App) -> anyhow::Result<()> {
        self.open(layout, cx)
    }
}

impl CardHost for BannerPresenter {
    fn act(&self, id: RowId, act: CardAct, cx: &mut App) {
        match act {
            CardAct::Open => self.run(id, |toast| toast.activation.clone(), cx),
            CardAct::Preview => self.run(id, |toast| toast.preview_action.clone(), cx),
            CardAct::Close => self.remove(id, cx),
        }
    }
}

fn same_message(held: &Toast, new: &Toast) -> bool {
    held.group == new.group
        && match (&held.key, &new.key) {
            (Some(held), Some(new)) => held == new,
            (None, None) => held.source == new.source,
            _ => false,
        }
}

struct Shown {
    row: SlabSnapshotRow,
    arrived: Instant,
    left: Option<Instant>,
    depth: pile::Tween,
}

struct BannerToastView {
    host: BannerPresenter,
    card: card::Host,
    size: Size<Pixels>,
    shown: Vec<Shown>,
    shape: InputShape,
    dismisser: SurfaceDismisser,
    inside: bool,
}

impl BannerToastView {
    fn new(host: BannerPresenter, dismisser: SurfaceDismisser, size: Size<Pixels>) -> Self {
        Self {
            card: Rc::new(host.clone()),
            host,
            size,
            shown: Vec::new(),
            shape: InputShape::default(),
            dismisser,
            inside: false,
        }
    }

    fn enter(&mut self, cx: &mut Context<Self>) {
        if self.inside {
            return;
        }
        self.inside = true;
        self.host.set_hovering(true, cx);
        let title = self.dismisser.current_title();
        self.shape.sense(
            title,
            |view: &mut Self| &mut view.shape,
            |view: &Self| view.dismisser.current_title(),
            |view, event, cx| match event {
                Some(InputEvent::Pointer(pointer)) if pointer.inside => {}
                Some(InputEvent::Escape) => {}
                _ => view.leave(cx),
            },
            cx,
        );
        cx.notify();
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        self.shape.rest();
        if !self.inside {
            return;
        }
        self.inside = false;
        self.host.set_hovering(false, cx);
        cx.notify();
    }

    fn settle(&mut self, rows: &[SlabSnapshotRow], now: Instant) {
        let count = rows.len();
        for (index, row) in rows.iter().enumerate() {
            let depth = (count - 1 - index) as f32;
            match self
                .shown
                .iter_mut()
                .find(|shown| shown.row.id == row.id && shown.left.is_none())
            {
                Some(shown) => {
                    shown.row = row.clone();
                    shown.depth.toward(depth, now);
                }
                None => self.shown.push(Shown {
                    row: row.clone(),
                    arrived: now,
                    left: None,
                    depth: pile::Tween::at(depth).gliding(),
                }),
            }
        }
        for shown in &mut self.shown {
            if shown.left.is_none() && rows.iter().all(|row| row.id != shown.row.id) {
                shown.left = Some(now);
            }
        }
        self.shown.retain(|shown| {
            shown
                .left
                .is_none_or(|left| now.saturating_duration_since(left) < Motion::LEAVE.duration)
        });
    }

    fn layer(
        &self,
        shown: &Shown,
        pose: Pose,
        front: bool,
        now: Instant,
        kit: crate::kit::Kit,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = &shown.row;
        let frame = pose.frame;
        let mut layer = div()
            .id(("toast-message", row.id.0))
            .absolute()
            .left(px(frame.left))
            .top(px(frame.top))
            .w(px(frame.width))
            .h(px(frame.height))
            .opacity(pose.opacity)
            .child(card::message(
                row,
                pose.scale,
                pose.content,
                card::counting(row, self.inside, now).unwrap_or(1.0),
                kit,
            ));
        if !front {
            return layer.into_any_element();
        }
        let id = row.id;
        let closer = self.card.clone();
        layer = layer
            .occlude()
            .on_mouse_move(cx.listener(|view, _: &MouseMoveEvent, _, cx| view.enter(cx)))
            .on_mouse_down(MouseButton::Middle, move |_, window, cx| {
                closer.act(id, CardAct::Close, cx);
                window.refresh();
            });
        if row.toast.activation.is_some() {
            let opener = self.card.clone();
            layer = layer
                .cursor_pointer()
                .on_click(move |_, _, cx| opener.act(id, CardAct::Open, cx));
        }
        layer.into_any_element()
    }
}

impl Render for BannerToastView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kit = crate::kit::kit();
        let now = Instant::now();
        let card = (f32::from(self.size.width), f32::from(self.size.height));
        let rows = self.host.snapshot();
        self.settle(&rows, now);

        let mut moving = false;
        let mut timing = false;
        let mut deepest: f32 = -1.0;
        let mut order: Vec<(f32, &Shown)> = self
            .shown
            .iter()
            .map(|shown| (shown.depth.value(now), shown))
            .collect();
        order.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut layers = Vec::with_capacity(order.len());
        for (depth, shown) in order {
            let pose = pose(depth, card, shown.arrived, shown.left, now);
            moving |= pose.moving || shown.depth.moving(now);
            deepest = deepest.max(shown.depth.target().max(depth));
            if pose.opacity > 0.0 {
                let front = shown.left.is_none() && shown.depth.target() == 0.0;
                timing |= front && !self.inside && shown.row.deadline.is_some();
                layers.push(self.layer(shown, pose, front, now, kit, cx));
            }
        }
        if moving || timing {
            window.request_animation_frame();
        }
        let reach = if deepest < 0.0 {
            0.0
        } else {
            card.1 + EDGE_STEP * deepest.min(MAX_EDGES as f32)
        };
        let title = self.dismisser.current_title();
        self.shape.reach(
            title,
            |view: &mut Self| &mut view.shape,
            [0.0, 0.0, card.0, reach],
            cx,
        );
        div().size_full().relative().children(layers)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pose {
    frame: Frame,
    scale: f32,
    opacity: f32,
    content: f32,
    moving: bool,
}

fn pose(
    depth: f32,
    card: (f32, f32),
    arrived: Instant,
    left: Option<Instant>,
    now: Instant,
) -> Pose {
    let (scale, opacity, moving) = message_pose(arrived, left, now);
    let edge = depth.clamp(0.0, MAX_EDGES as f32);
    let width = (card.0 - 2.0 * EDGE_INSET * edge) * scale;
    let height = card.1 * scale;
    Pose {
        frame: Frame {
            left: EDGE_INSET * edge + (card.0 - 2.0 * EDGE_INSET * edge - width) / 2.0,
            top: EDGE_STEP * edge + (card.1 - height) / 2.0,
            width,
            height,
        },
        scale,
        opacity,
        content: (1.0 - depth).clamp(0.0, 1.0),
        moving,
    }
}

fn message_pose(arrived: Instant, leaving: Option<Instant>, now: Instant) -> (f32, f32, bool) {
    let lerp = |from: f32, to: f32, t: f32| from + (to - from) * t;
    let grown = Motion::TRAVEL.progress(now.saturating_duration_since(arrived));
    let gone = leaving.map_or(0.0, |start| {
        Motion::LEAVE.progress(now.saturating_duration_since(start))
    });
    let scale = lerp(qol_theme::toast::MESSAGE_ARRIVE_SCALE, 1.0, grown)
        * lerp(1.0, qol_theme::toast::MESSAGE_LEAVE_SCALE, gone);
    let moving = grown < 1.0 || (leaving.is_some() && gone < 1.0);
    (scale, grown * (1.0 - gone), moving)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{message_pose, pose, same_message};
    use crate::toast::{Toast, ToastLayout};

    #[test]
    fn a_message_grows_in_and_shrinks_out() {
        let start = Instant::now();
        let at = |millis: u64| start + Duration::from_millis(millis);
        let leave = |millis: u64| Some(at(millis));
        let close = |(scale, opacity, moving): (f32, f32, bool), want: (f32, f32, bool)| {
            (scale - want.0).abs() < 1e-4 && (opacity - want.1).abs() < 1e-4 && moving == want.2
        };
        assert!(close(message_pose(start, None, at(0)), (0.96, 0.0, true)));
        let (scale, opacity, _) = message_pose(start, None, at(130));
        assert!(scale > 0.96 && scale < 1.0 && opacity > 0.0 && opacity < 1.0);
        assert!(close(message_pose(start, None, at(260)), (1.0, 1.0, false)));
        assert!(close(
            message_pose(start, leave(1000), at(1000)),
            (1.0, 1.0, true)
        ));
        let (scale, opacity, _) = message_pose(start, leave(1000), at(1060));
        assert!(scale < 1.0 && scale > 0.85 && opacity < 1.0 && opacity > 0.0);
        assert!(close(
            message_pose(start, leave(1000), at(1120)),
            (0.85, 0.0, false)
        ));
    }

    #[test]
    fn messages_behind_the_front_show_as_narrower_edges_below_it() {
        let settled = Instant::now() - Duration::from_secs(1);
        let now = Instant::now();
        let card = (440.0, 64.0);
        let at = |depth: f32| pose(depth, card, settled, None, now);
        let front = at(0.0);
        assert_eq!(
            (
                front.frame.left,
                front.frame.top,
                front.frame.width,
                front.frame.height
            ),
            (0.0, 0.0, 440.0, 64.0)
        );
        assert_eq!(front.content, 1.0);
        let second = at(1.0);
        assert_eq!(
            (second.frame.left, second.frame.top, second.frame.width),
            (12.0, 6.0, 416.0)
        );
        assert_eq!(second.content, 0.0);
        let third = at(2.0);
        assert_eq!((third.frame.left, third.frame.top), (24.0, 12.0));
        assert_eq!(at(5.0).frame, at(3.0).frame);
        assert!(!front.moving);
    }

    #[test]
    fn a_message_replaces_the_one_from_the_same_plugin_and_stacks_on_others() {
        let push = |group: &str, source: &str| {
            Toast::new("t", "m", ToastLayout::status())
                .group(group.to_string())
                .source(source.to_string())
        };
        let monitor = push("qol-monitor", "Display");
        assert!(same_message(&monitor, &push("qol-monitor", "Display")));
        assert!(!same_message(&monitor, &push("qol-bluetooth", "Bluetooth")));
        let countdown = push("qol-shot", "Shot").key("recording".to_string());
        assert!(same_message(
            &countdown,
            &push("qol-shot", "Shot").key("recording".to_string())
        ));
        assert!(!same_message(
            &countdown,
            &push("qol-shot", "Shot").key("saved".to_string())
        ));
        assert!(!same_message(&countdown, &push("qol-shot", "Shot")));
    }
}
