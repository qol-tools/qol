//! Identity-anchored selection over the attention-sorted session list.
//!
//! The panel re-sorts on every reconcile, so a session can change rows at any
//! moment - it flips to `NeedsYou` and jumps to the top, pushing everything
//! else down. Tracking the selection by row *index* therefore points at a
//! different session the instant the list reorders, which is how a click
//! "focused a completely wrong instance".
//!
//! This model anchors the selection to a `window_id`. The highlight and every
//! action (click, Enter, acknowledge) follow the session, never the slot it
//! happened to occupy. `order` is the window-id sequence currently on screen
//! (the sorted rows); the model is pure and holds no reference to it.

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    anchor: Option<u64>,
}

impl Selection {
    /// The window the selection is pinned to, if any. Not guaranteed to still
    /// be on screen - use [`Selection::resolved`] for the live target.
    pub fn anchored(&self) -> Option<u64> {
        self.anchor
    }

    /// Pin the selection to a specific session (a click, or a jump target).
    pub fn select(&mut self, window_id: u64) {
        self.anchor = Some(window_id);
    }

    /// Row to highlight in `order`: the anchored session's current position,
    /// or the first row when nothing is anchored or the anchor has left the
    /// list. `None` only when `order` is empty.
    pub fn highlight_index(&self, order: &[u64]) -> Option<usize> {
        if order.is_empty() {
            return None;
        }
        Some(
            self.anchor
                .and_then(|wid| order.iter().position(|&w| w == wid))
                .unwrap_or(0),
        )
    }

    /// The window the current selection resolves to for an action (Enter,
    /// acknowledge). Follows the same fallback as [`Selection::highlight_index`];
    /// `None` only when `order` is empty.
    pub fn resolved(&self, order: &[u64]) -> Option<u64> {
        self.highlight_index(order).map(|index| order[index])
    }

    /// Move one row down in the *current* `order`, re-anchoring to that window
    /// so the selection sticks to it through later reorders. Clamps at the end.
    pub fn move_down(&mut self, order: &[u64]) {
        self.step(order, 1);
    }

    /// Move one row up in the *current* `order`, re-anchoring to that window.
    /// Clamps at the top.
    pub fn move_up(&mut self, order: &[u64]) {
        self.step(order, -1);
    }

    fn step(&mut self, order: &[u64], delta: isize) {
        let Some(current) = self.highlight_index(order) else {
            self.anchor = None;
            return;
        };
        let last = order.len() as isize - 1;
        let next = (current as isize + delta).clamp(0, last) as usize;
        self.anchor = Some(order[next]);
    }
}
