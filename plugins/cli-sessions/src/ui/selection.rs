use qol_terminal_sessions::SessionId;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    anchor: Option<SessionId>,
}

impl Selection {
    pub fn select(&mut self, id: SessionId) {
        self.anchor = Some(id);
    }

    pub fn highlight_index(&self, order: &[SessionId]) -> Option<usize> {
        if order.is_empty() {
            return None;
        }
        Some(
            self.anchor
                .as_ref()
                .and_then(|id| order.iter().position(|candidate| candidate == id))
                .unwrap_or(0),
        )
    }

    pub fn resolved(&self, order: &[SessionId]) -> Option<SessionId> {
        self.highlight_index(order)
            .and_then(|index| order.get(index))
            .cloned()
    }

    pub fn move_down(&mut self, order: &[SessionId]) {
        self.step(order, 1);
    }

    pub fn move_up(&mut self, order: &[SessionId]) {
        self.step(order, -1);
    }

    fn step(&mut self, order: &[SessionId], delta: isize) {
        let Some(current) = self.highlight_index(order) else {
            self.anchor = None;
            return;
        };
        let last = order.len() as isize - 1;
        let next = (current as isize + delta).clamp(0, last) as usize;
        self.anchor = order.get(next).cloned();
    }
}
