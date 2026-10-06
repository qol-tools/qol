use std::cell::RefMut;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, AsyncApp};

use super::{RowId, SlabSnapshotRow, Toast};

pub(super) type Timer = (RowId, u64, Duration);

const HOLD_RECHECK: Duration = Duration::from_millis(400);

pub(super) struct Row {
    pub(super) id: RowId,
    pub(super) toast: Rc<Toast>,
    pub(super) generation: u64,
    pub(super) created: Instant,
    pub(super) deadline: Option<Instant>,
}

#[derive(Default)]
pub(super) struct Rows {
    list: Vec<Row>,
    next_id: u64,
    next_generation: u64,
}

impl Rows {
    pub(super) fn put(
        &mut self,
        toast: Toast,
        same: impl Fn(&Toast, &Toast) -> bool,
    ) -> (RowId, u64) {
        let generation = self.next_generation();
        let created = Instant::now();
        if let Some(row) = self.list.iter_mut().find(|row| same(&row.toast, &toast)) {
            row.generation = generation;
            row.toast = Rc::new(toast);
            row.created = created;
            row.deadline = None;
            return (row.id, generation);
        }
        let id = RowId(self.next_id);
        self.next_id += 1;
        self.list.push(Row {
            id,
            toast: Rc::new(toast),
            generation,
            created,
            deadline: None,
        });
        (id, generation)
    }

    pub(super) fn next_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.next_generation
    }

    pub(super) fn time_front(&mut self, now: Instant) -> Option<Timer> {
        let front = self.hold_behind()?;
        if self.list[front].deadline.is_some() {
            return None;
        }
        self.restart(front, now)
    }

    pub(super) fn restart_front(&mut self, now: Instant) -> Option<Timer> {
        let front = self.hold_behind()?;
        self.restart(front, now)
    }

    fn hold_behind(&mut self) -> Option<usize> {
        let front = self.list.len().checked_sub(1)?;
        for index in 0..front {
            if self.list[index].deadline.take().is_some() {
                self.list[index].generation = self.next_generation();
            }
        }
        Some(front)
    }

    fn restart(&mut self, index: usize, now: Instant) -> Option<Timer> {
        let timeout = self.list[index].toast.effective_timeout()?;
        let generation = self.next_generation();
        let row = &mut self.list[index];
        row.generation = generation;
        row.deadline = Some(now + timeout);
        Some((row.id, generation, timeout))
    }

    pub(super) fn owns(&self, id: RowId, generation: u64) -> bool {
        self.list
            .iter()
            .any(|row| row.id == id && row.generation == generation)
    }

    pub(super) fn raise(&mut self, id: RowId) {
        if let Some(index) = self.list.iter().position(|row| row.id == id) {
            let row = self.list.remove(index);
            self.list.push(row);
        }
    }

    pub(super) fn keep_newest(&mut self, most: usize) {
        let stale = self.list.len().saturating_sub(most);
        self.list.drain(..stale);
    }

    pub(super) fn remove(&mut self, ids: &[RowId]) {
        self.list.retain(|row| !ids.contains(&row.id));
    }

    pub(super) fn clear(&mut self) {
        self.list.clear();
    }

    pub(super) fn snapshot(&self) -> Vec<SlabSnapshotRow> {
        self.list
            .iter()
            .map(|row| SlabSnapshotRow {
                id: row.id,
                toast: row.toast.clone(),
                created: row.created,
                deadline: row.deadline,
            })
            .collect()
    }
}

impl Deref for Rows {
    type Target = [Row];

    fn deref(&self) -> &[Row] {
        &self.list
    }
}

impl DerefMut for Rows {
    fn deref_mut(&mut self) -> &mut [Row] {
        &mut self.list
    }
}

pub(super) trait Timed: Clone + 'static {
    fn rows(&self) -> RefMut<'_, Rows>;
    fn held(&self) -> bool;
    fn expire(&self, id: RowId, cx: &mut App);
}

pub(super) fn arm<T: Timed>(host: &T, timer: Option<Timer>, cx: &mut App) {
    let Some((id, generation, delay)) = timer else {
        return;
    };
    let host = host.clone();
    after(delay, cx, move |cx| {
        if !host.rows().owns(id, generation) {
            return;
        }
        if host.held() {
            arm(&host, Some((id, generation, HOLD_RECHECK)), cx);
        } else {
            host.expire(id, cx);
        }
    });
}

pub(super) fn after(delay: Duration, cx: &mut App, then: impl FnOnce(&mut App) + 'static) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(delay).await;
        let _ = cx.update(then);
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell, RefMut};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use gpui::{App, TestAppContext};

    use super::{arm, Rows, Timed};
    use crate::toast::{RowId, Toast, ToastLayout};

    #[derive(Clone, Default)]
    struct Host {
        rows: Rc<RefCell<Rows>>,
        held: Rc<Cell<bool>>,
        expired: Rc<Cell<usize>>,
    }

    impl Timed for Host {
        fn rows(&self) -> RefMut<'_, Rows> {
            self.rows.borrow_mut()
        }

        fn held(&self) -> bool {
            self.held.get()
        }

        fn expire(&self, id: RowId, _cx: &mut App) {
            self.rows.borrow_mut().remove(&[id]);
            self.expired.set(self.expired.get() + 1);
        }
    }

    fn toast(title: &str, key: Option<&str>) -> Toast {
        let toast = Toast::new(title.to_string(), "", ToastLayout::status()).group("plugin");
        match key {
            Some(key) => toast.key(key.to_string()),
            None => toast,
        }
    }

    fn keyed(held: &Toast, new: &Toast) -> bool {
        new.key.is_some() && held.group == new.group && held.key == new.key
    }

    fn titles(rows: &Rows) -> Vec<String> {
        rows.iter().map(|row| row.toast.title.to_string()).collect()
    }

    #[test]
    fn a_matching_toast_replaces_its_row_in_place_and_a_new_one_appends() {
        let mut rows = Rows::default();
        let (first, generation) = rows.put(toast("a", Some("k")), keyed);
        rows.put(toast("b", None), keyed);
        let (again, newer) = rows.put(toast("a2", Some("k")), keyed);
        assert!(first == again);
        assert!(newer > generation);
        assert!(!rows.owns(first, generation));
        assert!(rows.owns(first, newer));
        assert_eq!(titles(&rows), ["a2", "b"]);
    }

    #[test]
    fn raising_moves_a_row_to_the_newest_end_and_trimming_drops_the_oldest() {
        let mut rows = Rows::default();
        let (a, _) = rows.put(toast("a", None), keyed);
        rows.put(toast("b", None), keyed);
        rows.put(toast("c", None), keyed);
        rows.raise(a);
        assert_eq!(titles(&rows), ["b", "c", "a"]);
        rows.keep_newest(2);
        assert_eq!(titles(&rows), ["c", "a"]);
        rows.remove(&[a]);
        assert_eq!(titles(&rows), ["c"]);
    }

    #[test]
    fn only_the_front_row_counts_down_and_the_next_starts_when_it_leaves() {
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        let mut rows = Rows::default();
        rows.put(toast("a", None), keyed);
        let first = rows.time_front(now).expect("a info toast is timed");
        rows.put(toast("b", None), keyed);
        let second = rows.time_front(now).expect("the new front is timed");
        assert!(!rows.owns(first.0, first.1));
        assert_eq!(rows[0].deadline, None);
        assert_eq!(rows[1].deadline, Some(now + second.2));
        assert!(rows.time_front(later).is_none());
        rows.remove(&[second.0]);
        let promoted = rows.time_front(later).expect("the next row starts");
        assert!(promoted.0 == first.0);
        assert_eq!(rows[0].deadline, Some(later + promoted.2));
        let again = rows.restart_front(later + promoted.2).expect("restarted");
        assert!(!rows.owns(promoted.0, promoted.1));
        assert!(rows.owns(again.0, again.1));
    }

    #[gpui::test]
    fn a_held_front_waits_until_it_is_released(cx: &mut TestAppContext) {
        let host = Host::default();
        host.rows().put(toast("a", None), keyed);
        host.held.set(true);
        cx.update(|cx| {
            let timer = host.rows().time_front(Instant::now());
            arm(&host, timer, cx);
        });
        cx.executor().advance_clock(Duration::from_secs(30));
        cx.run_until_parked();
        assert_eq!(host.expired.get(), 0);
        host.held.set(false);
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert_eq!(host.expired.get(), 1);
        assert!(host.rows().is_empty());
    }
}
