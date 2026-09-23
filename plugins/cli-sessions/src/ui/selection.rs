use std::cell::RefCell;

use qol_gpui::scroll_list::ScrollList;
use qol_terminal_sessions::SessionId;

#[derive(Clone, Debug)]
pub struct Selection {
    anchor: Option<SessionId>,
    list: RefCell<ScrollList>,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            anchor: None,
            list: RefCell::new(ScrollList::new(1)),
        }
    }
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        self.anchor == other.anchor
    }
}

impl Eq for Selection {}

impl Selection {
    pub fn select(&mut self, id: SessionId) {
        self.anchor = Some(id);
    }

    pub fn highlight_index(&self, order: &[SessionId]) -> Option<usize> {
        self.restore(order)
    }

    pub fn resolved(&self, order: &[SessionId]) -> Option<SessionId> {
        self.restore(order)
            .and_then(|index| order.get(index))
            .cloned()
    }

    pub fn move_down(&mut self, order: &[SessionId]) {
        self.step(order, 1);
    }

    pub fn move_up(&mut self, order: &[SessionId]) {
        self.step(order, -1);
    }

    fn restore(&self, order: &[SessionId]) -> Option<usize> {
        let index = if order.is_empty() {
            None
        } else {
            Some(
                self.anchor
                    .as_ref()
                    .and_then(|id| order.iter().position(|candidate| candidate == id))
                    .unwrap_or(0),
            )
        };
        self.list.borrow_mut().selected = index.unwrap_or(0);
        index
    }

    fn step(&mut self, order: &[SessionId], delta: isize) {
        if self.restore(order).is_none() {
            self.anchor = None;
            return;
        }
        let selected = {
            let mut list = self.list.borrow_mut();
            if delta > 0 {
                list.move_down(order.len());
            } else {
                list.move_up();
            }
            list.selected
        };
        self.anchor = order.get(selected).cloned();
    }
}
