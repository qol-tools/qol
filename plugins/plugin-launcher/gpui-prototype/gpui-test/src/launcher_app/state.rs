use super::layout::HEADER_HEIGHT;

pub struct LauncherState {
    pub query: String,
    pub cursor: usize,
    pub selection_anchor: Option<usize>,
    pub selected: usize,
    pub window_height: f32,
}

impl LauncherState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            cursor: 0,
            selection_anchor: None,
            selected: 0,
            window_height: HEADER_HEIGHT,
        }
    }

    pub fn query_len(&self) -> usize {
        self.query.chars().count()
    }

    pub fn selected_range(&self) -> Option<(usize, usize)> {
        let anchor = self.selection_anchor?;
        if anchor == self.cursor {
            None
        } else {
            Some((anchor.min(self.cursor), anchor.max(self.cursor)))
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }
}
