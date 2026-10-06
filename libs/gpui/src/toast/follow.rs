use std::cell::RefMut;

use gpui::*;

use crate::monitor::{ActiveMonitor, MonitorTracker};

#[derive(Default)]
pub(super) struct Follow {
    monitor: Option<Bounds<Pixels>>,
    watching: bool,
    moving: bool,
}

impl Follow {
    pub(super) fn land(&mut self, tracker: &MonitorTracker) -> Option<ActiveMonitor> {
        self.moving = false;
        let monitor = tracker
            .snapshot_monitor()
            .or_else(|| tracker.snapshot_cursor().map(|(monitor, _)| monitor));
        self.monitor = monitor.as_ref().map(ActiveMonitor::bounds);
        monitor
    }

    pub(super) fn moving(&self) -> bool {
        self.moving
    }
}

pub(super) trait Follower: Clone + 'static {
    type Carry: 'static;

    fn follow(&self) -> RefMut<'_, Follow>;
    fn depart(&self, cx: &mut App) -> Option<Self::Carry>;
    fn empty(&self) -> bool;
    fn arrive(&self, carry: Self::Carry, cx: &mut App) -> anyhow::Result<()>;
}

pub(super) fn watch<F: Follower>(follower: &F, cx: &mut App) {
    if std::mem::replace(&mut follower.follow().watching, true) {
        return;
    }
    let follower = follower.clone();
    crate::event_router::spawn_runtime_event_router(
        cx,
        vec![crate::protocol::RuntimeEventKind::ActiveMonitorChanged],
        move |cx, event| {
            let monitor = ActiveMonitor::from_event(event).map(|monitor| monitor.bounds());
            travel(&follower, monitor, cx);
        },
    );
}

fn travel<F: Follower>(follower: &F, monitor: Option<Bounds<Pixels>>, cx: &mut App) {
    let was = {
        let mut follow = follower.follow();
        if monitor.is_none() || follow.monitor == monitor {
            return;
        }
        std::mem::replace(&mut follow.moving, true)
    };
    let Some(carry) = follower.depart(cx) else {
        follower.follow().moving = was;
        return;
    };
    let follower = follower.clone();
    cx.defer(move |cx| {
        if follower.empty() {
            follower.follow().moving = false;
            return;
        }
        if let Err(error) = follower.arrive(carry, cx) {
            log::warn!("[toast] could not move to the active monitor: {error:#}");
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::{RefCell, RefMut};
    use std::rc::Rc;

    use gpui::{point, px, size, App, Bounds, Pixels, TestAppContext};

    use super::{travel, Follow, Follower};

    #[derive(Clone, Default)]
    struct Fake {
        follow: Rc<RefCell<Follow>>,
        open: Rc<RefCell<bool>>,
        rows: Rc<RefCell<usize>>,
        log: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Follower for Fake {
        type Carry = ();

        fn follow(&self) -> RefMut<'_, Follow> {
            self.follow.borrow_mut()
        }

        fn depart(&self, _cx: &mut App) -> Option<()> {
            if !std::mem::replace(&mut *self.open.borrow_mut(), false) {
                return None;
            }
            self.log.borrow_mut().push("depart");
            Some(())
        }

        fn empty(&self) -> bool {
            *self.rows.borrow() == 0
        }

        fn arrive(&self, _carry: (), _cx: &mut App) -> anyhow::Result<()> {
            *self.open.borrow_mut() = true;
            self.follow.borrow_mut().moving = false;
            self.log.borrow_mut().push("arrive");
            Ok(())
        }
    }

    fn monitor(x: f32) -> Option<Bounds<Pixels>> {
        Some(Bounds::new(
            point(px(x), px(0.0)),
            size(px(1920.0), px(1080.0)),
        ))
    }

    fn opened_on(at: f32, rows: usize) -> Fake {
        let fake = Fake::default();
        *fake.open.borrow_mut() = true;
        *fake.rows.borrow_mut() = rows;
        fake.follow.borrow_mut().monitor = monitor(at);
        fake
    }

    #[gpui::test]
    fn an_open_surface_moves_only_when_the_active_monitor_changes(cx: &mut TestAppContext) {
        let fake = opened_on(0.0, 1);
        cx.update(|cx| travel(&fake, monitor(0.0), cx));
        cx.update(|cx| travel(&fake, None, cx));
        assert!(fake.log.borrow().is_empty());
        cx.update(|cx| travel(&fake, monitor(1920.0), cx));
        assert_eq!(*fake.log.borrow(), ["depart", "arrive"]);
        assert!(!fake.follow.borrow().moving());
    }

    #[gpui::test]
    fn a_surface_that_empties_while_moving_stays_closed(cx: &mut TestAppContext) {
        let fake = opened_on(0.0, 0);
        cx.update(|cx| travel(&fake, monitor(1920.0), cx));
        assert_eq!(*fake.log.borrow(), ["depart"]);
        assert!(!*fake.open.borrow());
        assert!(!fake.follow.borrow().moving());
    }

    #[gpui::test]
    fn nothing_moves_while_no_surface_is_open(cx: &mut TestAppContext) {
        let fake = opened_on(0.0, 1);
        *fake.open.borrow_mut() = false;
        cx.update(|cx| travel(&fake, monitor(1920.0), cx));
        assert!(fake.log.borrow().is_empty());
        assert!(!fake.follow.borrow().moving());
    }
}
